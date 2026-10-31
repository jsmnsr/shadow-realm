use super::*;
use crate::cli::parse_args;
use std::fs;
use windows_sys::Win32::System::Ioctl::{
    GPT_BASIC_DATA_ATTRIBUTE_HIDDEN as HIDDEN,
    GPT_BASIC_DATA_ATTRIBUTE_NO_DRIVE_LETTER as NO_DRIVE_LETTER,
};

const ORIGINAL: u64 = HIDDEN | NO_DRIVE_LETTER | SHADOW_COPY | READ_ONLY;
const TRANSITIONS: [&str; 10] = [
    "lock", "dismount", "set", "unlock", "reopen", "lock", "dismount", "set", "unlock", "reopen",
];

struct MockVolume {
    attributes: u64,
    events: Vec<&'static str>,
}

impl Default for MockVolume {
    fn default() -> Self {
        Self {
            attributes: ORIGINAL,
            events: Vec::new(),
        }
    }
}

impl Volume for MockVolume {
    fn attributes(&mut self) -> Result<u64> {
        Ok(self.attributes)
    }
    fn set_attributes(&mut self, attributes: u64) -> Result<()> {
        self.events.push("set");
        self.attributes = attributes;
        Ok(())
    }
    fn is_writable(&mut self) -> Result<bool> {
        Ok(self.attributes & READ_ONLY == 0)
    }
    fn lock(&mut self) -> Result<()> {
        self.events.push("lock");
        Ok(())
    }
    fn dismount(&mut self) -> Result<()> {
        self.events.push("dismount");
        Ok(())
    }
    fn unlock(&mut self) -> Result<()> {
        self.events.push("unlock");
        Ok(())
    }
    fn reopen(&mut self) -> Result<()> {
        self.events.push("reopen");
        Ok(())
    }
}

fn execute(volume: &mut MockVolume, root: &Path, args: &[&str]) -> Result<()> {
    let options = parse_args(&args.iter().map(ToString::to_string).collect::<Vec<_>>())?;
    run_file_command(volume, root.to_str().unwrap(), &options)
}

#[test]
fn crud_dispatch_remounts_only_mutations_and_preserves_read_bytes() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let source = root.join("source");
    let output = root.join("output");
    fs::write(&source, b"\0\xffbinary\r\n").unwrap();
    let mut volume = MockVolume::default();
    for command in ["create", "update", "delete"] {
        let mut args = vec![command, "--snapshot", "3", "--file", "target"];
        if command != "delete" {
            args.extend(["--source", source.to_str().unwrap()]);
        }
        let mut preview = args.clone();
        preview.push("--dry-run");
        execute(&mut volume, root, &preview).unwrap();
        assert_eq!(volume.events, Vec::<&str>::new());
        assert_eq!(root.join("target").exists(), command != "create");
        if command == "update" {
            assert_eq!(fs::read(root.join("target")).unwrap(), b"\0\xffbinary\r\n");
        }
        args.push("--confirm-lab");
        if command == "delete" {
            args.push("--confirm-delete");
        }
        execute(&mut volume, root, &args).unwrap();
        assert_eq!(volume.attributes, ORIGINAL);
        assert_eq!(volume.events, TRANSITIONS);
        volume.events.clear();
        if command == "delete" {
            assert!(!root.join("target").exists());
        } else {
            execute(
                &mut volume,
                root,
                &[
                    "read",
                    "--snapshot",
                    "3",
                    "--file",
                    "target",
                    "--output",
                    output.to_str().unwrap(),
                ],
            )
            .unwrap();
            assert_eq!(volume.events, Vec::<&str>::new());
            assert_eq!(fs::read(&output).unwrap(), fs::read(&source).unwrap());
            fs::remove_file(&output).unwrap();
            fs::write(&source, b"short").unwrap();
        }
    }
    area.close().unwrap();
}

#[test]
fn missing_source_fails_before_remounting() {
    let area = tempfile::tempdir().unwrap();
    let mut volume = MockVolume::default();
    let missing = area.path().join("missing");
    assert!(
        execute(
            &mut volume,
            area.path(),
            &[
                "create",
                "--snapshot",
                "3",
                "--file",
                "target",
                "--source",
                missing.to_str().unwrap(),
                "--confirm-lab"
            ]
        )
        .is_err()
    );
    assert_eq!(volume.events, Vec::<&str>::new());
    assert!(!area.path().join("target").exists());
    area.close().unwrap();
}

#[test]
fn failed_update_still_remounts_readonly() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let source = root.join("source");
    let target = root.join("target");
    fs::write(&source, b"replacement").unwrap();
    fs::write(&target, b"keep").unwrap();
    let permissions = fs::metadata(&target).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&target, readonly).unwrap();
    let mut volume = MockVolume::default();
    assert!(
        execute(
            &mut volume,
            root,
            &[
                "update",
                "--snapshot",
                "3",
                "--file",
                "target",
                "--source",
                source.to_str().unwrap(),
                "--confirm-lab"
            ]
        )
        .is_err()
    );
    assert_eq!(volume.attributes, ORIGINAL);
    assert_eq!(volume.events, TRANSITIONS);
    assert_eq!(fs::read(&target).unwrap(), b"keep");
    fs::set_permissions(&target, permissions).unwrap();
    area.close().unwrap();
}
