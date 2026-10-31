use super::*;
use windows_sys::Win32::Foundation::GENERIC_WRITE;
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_WRITE};

#[test]
fn native_timestamps_preserve_both_halves() {
    let time = FILETIME {
        dwLowDateTime: 0x0123_4567,
        dwHighDateTime: 0x89ab_cdef,
    };
    assert_eq!(filetime_ticks(time), 0x89ab_cdef_0123_4567);
}

#[test]
fn preview_is_read_only_and_deletion_removes_only_the_selected_file() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    fs::create_dir(root.join("nested")).unwrap();
    let target = root.join("nested").join("data.bat");
    fs::write(&target, b"test-only content").unwrap();
    let sentinel = root.join("keep.txt");
    fs::write(&sentinel, b"unchanged").unwrap();
    let name = root.to_str().unwrap();
    let id = preview(name, r"nested\data.bat").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"test-only content");
    delete_file(name, r"nested\data.bat", &id).unwrap();
    assert!(!target.exists());
    assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
    area.close().unwrap();
}

#[test]
fn changed_target_and_directories_are_not_deleted() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let target = root.join("target");
    fs::write(&target, b"before").unwrap();
    let name = root.to_str().unwrap();
    let id = preview(name, "target").unwrap();
    fs::write(&target, b"changed content").unwrap();
    assert!(delete_file(name, "target", &id).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"changed content");
    fs::create_dir(root.join("folder")).unwrap();
    assert!(preview(name, "folder").is_err());
    assert!(preview(name, "missing").is_err());
    area.close().unwrap();
}

#[test]
fn busy_and_readonly_files_are_not_forced_and_attributes_are_preserved() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let target = root.join("target");
    fs::write(&target, b"keep").unwrap();
    let name = root.to_str().unwrap();
    let id = preview(name, "target").unwrap();
    let busy = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&target)
        .unwrap();
    assert!(preview(name, "target").is_err());
    assert!(delete_file(name, "target", &id).is_err());
    drop(busy);
    let original_permissions = fs::metadata(&target).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&target, permissions).unwrap();
    assert!(delete_file(name, "target", &id).is_err());
    assert!(fs::metadata(&target).unwrap().permissions().readonly());
    fs::set_permissions(&target, original_permissions).unwrap();
    area.close().unwrap();
}

#[test]
fn pinned_parent_cannot_be_renamed_during_the_operation() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    fs::create_dir(root.join("parent")).unwrap();
    fs::write(root.join("parent").join("file"), b"keep").unwrap();
    let checked = open_checked(root.to_str().unwrap(), r"parent\file", 0).unwrap();
    assert!(fs::rename(root.join("parent"), root.join("moved")).is_err());
    assert!(
        open_file_handle(
            &root.join("parent"),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            OPEN_REPARSE_BACKUP
        )
        .is_err()
    );
    drop(checked);
    drop(
        open_file_handle(
            &root.join("parent"),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            OPEN_REPARSE_BACKUP,
        )
        .unwrap(),
    );
    area.close().unwrap();
}

#[test]
fn junction_cannot_redirect_deletion_outside_the_selected_root() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let outside = tempfile::tempdir().unwrap();
    let outside_root = outside.path();
    let sentinel = outside_root.join("sentinel");
    fs::write(&sentinel, b"outside").unwrap();
    let junction = root.join("junction");
    let output = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside_root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        preview(root.to_str().unwrap(), r"junction\sentinel")
            .unwrap_err()
            .chain()
            .any(|cause| cause.to_string().contains("reparse"))
    );
    assert!(
        preview(root.to_str().unwrap(), "junction")
            .unwrap_err()
            .chain()
            .any(|cause| cause.to_string().contains("reparse"))
    );
    let outside_id = preview(outside_root.to_str().unwrap(), "sentinel").unwrap();
    assert!(delete_file(root.to_str().unwrap(), r"junction\sentinel", &outside_id).is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside");
    fs::remove_dir(junction).unwrap();
    area.close().unwrap();
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside");
    outside.close().unwrap();
}

#[test]
fn replacement_with_same_contents_is_not_deleted() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let target = root.join("target");
    fs::write(&target, b"same content").unwrap();
    let id = preview(root.to_str().unwrap(), "target").unwrap();
    fs::rename(&target, root.join("original")).unwrap();
    fs::write(&target, b"same content").unwrap();
    assert!(delete_file(root.to_str().unwrap(), "target", &id).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"same content");
    area.close().unwrap();
}

#[test]
fn deleting_one_hard_link_does_not_erase_the_other_name() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let target = root.join("target");
    let alias = root.join("alias");
    fs::write(&target, b"shared data").unwrap();
    fs::hard_link(&target, &alias).unwrap();
    let id = preview(root.to_str().unwrap(), "target").unwrap();
    delete_file(root.to_str().unwrap(), "target", &id).unwrap();
    assert!(!target.exists());
    assert_eq!(fs::read(&alias).unwrap(), b"shared data");
    area.close().unwrap();
}

#[test]
fn binary_create_read_and_update_round_trip_including_empty_and_shorter_files() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    fs::create_dir(root.join("nested")).unwrap();
    let name = root.to_str().unwrap();
    let payload: Vec<_> = (0_u8..=255).cycle().take(150_000).collect();
    let expected = preview_create(name, r"nested\data.bin").unwrap();
    create_file(name, r"nested\data.bin", &expected, &payload).unwrap();
    assert_eq!(fs::read(root.join(r"nested\data.bin")).unwrap(), payload);
    for bytes in [&b"\0\xffshort"[..], &b""[..], &payload[..]] {
        let id = preview_update(name, r"nested\data.bin").unwrap();
        update_file(name, r"nested\data.bin", &id, bytes).unwrap();
        let mut observed = Vec::new();
        copy_contents(
            open_checked(name, r"nested\data.bin", 0).unwrap(),
            &mut observed,
        )
        .unwrap();
        assert_eq!(observed, bytes);
    }
    let output = root.join("export.bin");
    read_file(name, r"nested\data.bin", Some(&output)).unwrap();
    assert_eq!(fs::read(&output).unwrap(), payload);
    assert!(read_file(name, r"nested\data.bin", Some(&output)).is_err());
    assert_eq!(fs::read(&output).unwrap(), payload);
    let empty = preview_create(name, "empty").unwrap();
    create_file(name, "empty", &empty, &[]).unwrap();
    assert_eq!(fs::read(root.join("empty")).unwrap(), Vec::<u8>::new());
    area.close().unwrap();
}

#[test]
fn create_never_overwrites_existing_names_and_rechecks_parent_identity() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let name = root.to_str().unwrap();
    fs::create_dir(root.join("parent")).unwrap();
    let expected = preview_create(name, r"parent\new").unwrap();
    fs::write(root.join(r"parent\new"), b"keep").unwrap();
    assert!(preview_create(name, r"parent\new").is_err());
    assert!(preview_create(name, "parent").is_err());
    assert!(create_file(name, r"parent\new", &expected, b"overwrite").is_err());
    assert_eq!(fs::read(root.join(r"parent\new")).unwrap(), b"keep");
    fs::remove_file(root.join(r"parent\new")).unwrap();
    fs::rename(root.join("parent"), root.join("original")).unwrap();
    fs::create_dir(root.join("parent")).unwrap();
    assert!(
        create_file(name, r"parent\new", &expected, b"wrong parent")
            .unwrap_err()
            .to_string()
            .contains("identity")
    );
    assert!(!root.join(r"parent\new").exists());
    assert!(preview_create(name, r"missing\new").is_err());
    assert!(preview_create(name, r"..\escape").is_err());
    area.close().unwrap();
}

#[test]
fn update_requires_unchanged_existing_single_link_file() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let name = root.to_str().unwrap();
    let target = root.join("target");
    fs::write(&target, b"before").unwrap();
    let id = preview_update(name, "target").unwrap();
    fs::write(&target, b"changed contents").unwrap();
    assert!(update_file(name, "target", &id, b"replacement").is_err());
    assert_eq!(fs::read(&target).unwrap(), b"changed contents");
    let id = preview_update(name, "target").unwrap();
    fs::rename(&target, root.join("original")).unwrap();
    fs::write(&target, b"changed contents").unwrap();
    assert!(update_file(name, "target", &id, b"replacement").is_err());
    assert_eq!(fs::read(&target).unwrap(), b"changed contents");
    let id = preview_update(name, "target").unwrap();
    fs::hard_link(&target, root.join("alias")).unwrap();
    assert!(
        preview_update(name, "target")
            .unwrap_err()
            .to_string()
            .contains("hard link")
    );
    assert!(
        update_file(name, "target", &id, b"replacement")
            .unwrap_err()
            .to_string()
            .contains("hard link")
    );
    assert_eq!(fs::read(root.join("alias")).unwrap(), b"changed contents");
    assert!(update_file(name, "missing", &id, b"replacement").is_err());
    assert!(!root.join("missing").exists());
    assert!(preview_update(name, "missing").is_err());
    fs::create_dir(root.join("folder")).unwrap();
    assert!(preview_update(name, "folder").is_err());
    assert!(read_file(name, "folder", Some(&root.join("output"))).is_err());
    assert!(!root.join("output").exists());
    area.close().unwrap();
}

#[test]
fn update_and_read_respect_busy_and_readonly_files() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let name = root.to_str().unwrap();
    let target = root.join("target");
    fs::write(&target, b"keep").unwrap();
    let id = preview_update(name, "target").unwrap();
    let busy = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&target)
        .unwrap();
    assert!(update_file(name, "target", &id, b"replace").is_err());
    assert!(read_file(name, "target", Some(&root.join("output"))).is_err());
    drop(busy);
    let original = fs::metadata(&target).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&target, readonly).unwrap();
    assert!(update_file(name, "target", &id, b"replace").is_err());
    read_file(name, "target", Some(&root.join("output"))).unwrap();
    assert!(fs::metadata(&target).unwrap().permissions().readonly());
    assert_eq!(fs::read(&target).unwrap(), b"keep");
    fs::set_permissions(&target, original).unwrap();
    area.close().unwrap();
}

#[test]
fn junction_cannot_redirect_create_read_or_update() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let outside = tempfile::tempdir().unwrap();
    let outside_root = outside.path();
    let name = root.to_str().unwrap();
    let junction = root.join("junction");
    fs::create_dir(&junction).unwrap();
    let new = preview_create(name, r"junction\new").unwrap();
    fs::remove_dir(&junction).unwrap();
    fs::write(outside_root.join("sentinel"), b"outside").unwrap();
    let output = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside_root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(preview_create(name, r"junction\new").is_err());
    assert!(create_file(name, r"junction\new", &new, b"escape").is_err());
    assert!(!outside_root.join("new").exists());
    assert!(read_file(name, r"junction\sentinel", Some(&root.join("output"))).is_err());
    assert!(!root.join("output").exists());
    let id = preview_update(outside_root.to_str().unwrap(), "sentinel").unwrap();
    assert!(update_file(name, r"junction\sentinel", &id, b"escape").is_err());
    assert_eq!(fs::read(outside_root.join("sentinel")).unwrap(), b"outside");
    fs::remove_dir(junction).unwrap();
    area.close().unwrap();
    assert_eq!(fs::read(outside_root.join("sentinel")).unwrap(), b"outside");
    outside.close().unwrap();
}

#[test]
fn source_is_regular_binary_data_and_is_closed_after_loading() {
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    let source = root.join("source");
    fs::write(&source, b"\0\xffsource").unwrap();
    assert_eq!(load_source(&source).unwrap(), b"\0\xffsource");
    assert!(load_source(root).is_err());
    assert!(load_source(&root.join("missing")).is_err());
    area.close().unwrap();
}

#[test]
fn read_reports_output_failures_and_releases_all_handles() {
    struct FailingOutput;
    impl Write for FailingOutput {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("simulated output failure"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let area = tempfile::tempdir().unwrap();
    let root = area.path();
    fs::write(root.join("target"), b"contents").unwrap();
    let checked = open_checked(root.to_str().unwrap(), "target", 0).unwrap();
    let error = copy_contents(checked, &mut FailingOutput).unwrap_err();
    assert!(format!("{error:#}").contains("output failure"));
    area.close().unwrap();
}
