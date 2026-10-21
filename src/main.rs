use std::env;
use std::process::ExitCode;

#[cfg(windows)]
mod deletion;
#[cfg(windows)]
mod windows;

type Result<T> = std::result::Result<T, String>;

const NO_DRIVE_LETTER: u64 = 0x8000_0000_0000_0000;
const HIDDEN: u64 = 0x4000_0000_0000_0000;
const SHADOW_COPY: u64 = 0x2000_0000_0000_0000;
const READ_ONLY: u64 = 0x1000_0000_0000_0000;
const KNOWN_ATTRIBUTES: u64 = NO_DRIVE_LETTER | HIDDEN | SHADOW_COPY | READ_ONLY;

const HELP: &str = concat!(
    "\
VSS write-state experiment ",
    env!("CARGO_PKG_VERSION"),
    " -- DISPOSABLE WINDOWS VM ONLY

  vss-write-demo inspect --snapshot NUMBER
  vss-write-demo diagnose-lock --snapshot NUMBER --file RELATIVE_PATH --confirm-lab
  vss-write-demo probe --snapshot NUMBER --confirm-lab --remount-snapshot
  vss-write-demo restore-readonly --snapshot NUMBER --confirm-lab --remount-snapshot
  vss-write-demo delete --snapshot NUMBER --file RELATIVE_PATH --dry-run
  vss-write-demo delete --snapshot NUMBER --file RELATIVE_PATH --confirm-lab --confirm-delete
                       --remount-snapshot

inspect: query attributes and the driver's writable status; no mutations.
diagnose-lock: compare lock/unlock before and after previewing an existing file.
               No attribute changes, dismounts, or deletion. Locking can flush
               filesystem caches and briefly excludes other access.
probe: temporarily request writable attributes, create/read/delete one unique
       test file in the snapshot root, then restore the original attributes.
restore-readonly: recovery action; set READ_ONLY while preserving other attributes.
delete: remove exactly one existing regular-file name inside the selected snapshot.
        Requires --file and either --dry-run or BOTH confirmations. No wildcards.
--remount-snapshot: lock and dismount ONLY the selected shadow device around
                   each attribute change, then close/reopen its handle.
                   Never force-dismount a busy snapshot; lock failure stops it.
                   Required for all mutations, including read-only recovery.
Attribute-only restoration was removed after lab NTFS flush failures.

NUMBER is the numeric suffix of HarddiskVolumeShadowCopyNUMBER, NOT a VSS GUID.
Only exact shadow-device targets are accepted; source disks and paths are not.
Being the newest snapshot is NOT sufficient to guarantee write support.
Run elevated. Do not use production snapshots or host-mounted production disks.
Driver read-only status does NOT prove filesystem integrity or full recovery.
Restoration is best-effort on failure and is NOT guaranteed after termination,
power loss, a crash, or a driver error. File cleanup is not a full rollback.
"
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Inspect,
    Probe,
    RestoreReadonly,
    Delete,
    DiagnoseLock,
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    command: Command,
    snapshot: u32,
    remount: bool,
    file: Option<String>,
    dry_run: bool,
}

fn parse_args(args: &[String]) -> Result<Option<Options>> {
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        return Ok(None);
    }
    let command = match args[0].as_str() {
        "inspect" => Command::Inspect,
        "probe" => Command::Probe,
        "restore-readonly" => Command::RestoreReadonly,
        "delete" => Command::Delete,
        "diagnose-lock" => Command::DiagnoseLock,
        other => return Err(format!("Unknown command: {other}")),
    };
    let mut snapshot = None;
    let mut confirmed = false;
    let mut remount = false;
    let mut file = None;
    let mut dry_run = false;
    let mut confirm_delete = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--snapshot" => {
                if snapshot.is_some() {
                    return Err("--snapshot may only be specified once".into());
                }
                index += 1;
                let value = args.get(index).ok_or("--snapshot requires a number")?;
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err("--snapshot must be a positive decimal number, not a path".into());
                }
                let number = value
                    .parse::<u32>()
                    .map_err(|_| "--snapshot is outside the u32 range")?;
                if number == 0 {
                    return Err("--snapshot must be greater than zero".into());
                }
                snapshot = Some(number);
            }
            "--confirm-lab" if !confirmed => confirmed = true,
            "--remount-snapshot" if !remount => remount = true,
            "--restore-attributes-only" => {
                return Err("--restore-attributes-only was removed after NTFS flush failures. Attribute flags alone do not establish clean filesystem recovery. Use --remount-snapshot; if restoration fails, revert the disposable VM checkpoint.".into());
            }
            "--file" if file.is_none() => {
                index += 1;
                let value = args.get(index).ok_or("--file requires a relative path")?;
                validate_relative_file(value)?;
                file = Some(value.clone());
            }
            "--dry-run" if !dry_run => dry_run = true,
            "--confirm-delete" if !confirm_delete => confirm_delete = true,
            argument => return Err(format!("Unexpected or duplicate argument: {argument}")),
        }
        index += 1;
    }
    let snapshot = snapshot.ok_or("--snapshot is required")?;
    if command == Command::Delete {
        if file.is_none() {
            return Err("delete requires --file with a snapshot-relative path".into());
        }
        if dry_run && (confirmed || confirm_delete || remount) {
            return Err("--dry-run cannot be combined with mutation or confirmation flags".into());
        }
        if !dry_run && !confirm_delete {
            return Err(
                "Deletion requires --confirm-delete; use --dry-run to preview the target".into(),
            );
        }
    } else if command == Command::DiagnoseLock {
        if file.is_none() {
            return Err("diagnose-lock requires --file with a snapshot-relative path".into());
        }
        if dry_run || confirm_delete || remount {
            return Err("diagnose-lock does not accept deletion, dry-run, or remount flags".into());
        }
    } else if file.is_some() || dry_run || confirm_delete {
        return Err("--file requires delete/diagnose-lock; deletion flags require delete".into());
    }
    if matches!(
        command,
        Command::Probe | Command::RestoreReadonly | Command::Delete
    ) && !dry_run
        && !remount
    {
        return Err("Mutations require --remount-snapshot. Attribute-only transitions are no longer supported after lab NTFS flush failures.".into());
    }
    if command != Command::Inspect && !dry_run && !confirmed {
        return Err(
            "Mutation and locking commands require --confirm-lab; use a disposable VM".into(),
        );
    }
    if command == Command::Inspect && (confirmed || remount) {
        return Err("inspect does not use --confirm-lab or --remount-snapshot".into());
    }
    Ok(Some(Options {
        command,
        snapshot,
        remount,
        file,
        dry_run,
    }))
}

fn snapshot_path(number: u32) -> String {
    format!(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy{number}")
}

fn validate_relative_file(path: &str) -> Result<()> {
    if path.is_empty() || path.encode_utf16().count() > 30_000 {
        return Err(
            "--file must be a nonempty snapshot-relative path shorter than 30001 UTF-16 units"
                .into(),
        );
    }
    if path
        .chars()
        .any(|ch| ch.is_control() || "/:*?\"<>|".contains(ch))
    {
        return Err(
            "--file cannot contain a drive, stream, wildcard, control character, or forward slash"
                .into(),
        );
    }
    for component in path.split('\\') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
        {
            return Err("--file must not be rooted, contain traversal/empty components, or end a component with a dot/space".into());
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.chars().count() == 1
                    && suffix
                        .chars()
                        .next()
                        .is_some_and(|ch| matches!(ch, '1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'))
            })
        }) {
            return Err("--file cannot contain reserved DOS device names".into());
        }
    }
    Ok(())
}

trait Volume {
    fn attributes(&mut self) -> Result<u64>;
    fn set_attributes(&mut self, attributes: u64) -> Result<()>;
    fn is_writable(&mut self) -> Result<bool>;
    fn lock(&mut self) -> Result<()>;
    fn dismount(&mut self) -> Result<()>;
    fn unlock(&mut self) -> Result<()>;
    fn reopen(&mut self) -> Result<()>;
}

fn check_snapshot_attributes(attributes: u64) -> Result<()> {
    if attributes & (SHADOW_COPY | NO_DRIVE_LETTER) != (SHADOW_COPY | NO_DRIVE_LETTER) {
        return Err(
            "Driver did not report the expected shadow-copy attributes; refusing mutation".into(),
        );
    }
    if attributes & !KNOWN_ATTRIBUTES != 0 {
        return Err(format!(
            "Unrecognized attribute bits in {attributes:#018x}; refusing mutation"
        ));
    }
    Ok(())
}

fn combine(operation: Result<()>, cleanup: Result<()>, label: &str) -> Result<()> {
    match (operation, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(format!("{label}: {error}")),
        (Err(error), Err(cleanup_error)) => Err(format!("{error}\n{label}: {cleanup_error}")),
    }
}

fn diagnostic_lock<V: Volume>(
    volume: &mut V,
    stage: &str,
    failures: &mut Vec<String>,
) -> Result<()> {
    println!("{stage}: attempting FSCTL_LOCK_VOLUME");
    match volume.lock() {
        Ok(()) => {
            println!("{stage}: LOCK SUCCEEDED");
            volume.unlock().map_err(|error| {
                format!("{stage}: UNLOCK FAILED: {error}; stopping. The volume handle closes on command exit.")
            })?;
            println!("{stage}: UNLOCK SUCCEEDED");
        }
        Err(error) => {
            let message = format!("{stage}: LOCK FAILED: {error}");
            eprintln!("{message}");
            failures.push(message);
        }
    }
    Ok(())
}

fn diagnose_lock<V: Volume>(volume: &mut V, preview: impl FnOnce() -> Result<()>) -> Result<()> {
    let original = volume.attributes()?;
    check_snapshot_attributes(original)?;
    if original & READ_ONLY == 0 || volume.is_writable()? {
        return Err("Lock diagnosis requires an initially read-only snapshot".into());
    }
    println!(
        "Lock-only diagnosis: no attribute changes, dismounts, or deletion will be requested."
    );
    let operation = (|| {
        let mut failures = Vec::new();
        diagnostic_lock(volume, "BEFORE PREVIEW", &mut failures)?;
        println!(
            "PREVIEW: opening the selected file and its parents, then closing all preview handles."
        );
        if let Err(error) = preview() {
            failures.push(format!("PREVIEW FAILED: {error}"));
            return Err(failures.join("\n"));
        }
        // The same volume handle is used on both sides of the preview.
        if let Err(error) = diagnostic_lock(volume, "AFTER PREVIEW", &mut failures) {
            failures.push(error);
            return Err(failures.join("\n"));
        }
        if failures.is_empty() {
            println!("Both lock/unlock checks succeeded. This does not test write enablement or deletion.");
            Ok(())
        } else {
            Err(failures.join("\n"))
        }
    })();
    let verification = (|| {
        let observed = volume.attributes()?;
        if observed != original || volume.is_writable()? {
            return Err(format!(
                "Snapshot state changed during diagnosis: attributes {observed:#018x}; no automatic restoration was attempted"
            ));
        }
        println!("Driver attributes unchanged: {observed:#018x}; driver remains read-only.");
        Ok(())
    })();
    combine(operation, verification, "STATE VERIFICATION FAILED")
}

struct RemountGuard<'a, V: Volume> {
    volume: &'a mut V,
    armed: bool,
}

impl<V: Volume> RemountGuard<'_, V> {
    fn finish(&mut self) -> Result<()> {
        self.armed = false;
        let unlock = self.volume.unlock();
        let reopen = self.volume.reopen();
        combine(unlock, reopen, "SNAPSHOT REOPEN FAILED")
    }
}

impl<V: Volume> Drop for RemountGuard<'_, V> {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self.finish() {
                eprintln!("REMOUNT CLEANUP FAILED during unwinding: {error}");
            }
        }
    }
}

fn change_attributes<V: Volume>(volume: &mut V, attributes: u64, remount: bool) -> Result<()> {
    if !remount {
        return Err("Attribute-only transitions are disabled; remount consent is required".into());
    }
    println!("Locking the selected shadow device; busy snapshots will not be force-dismounted.");
    volume.lock()?;
    let mut guard = RemountGuard {
        volume,
        armed: true,
    };
    let operation = (|| {
        println!("Dismounting the locked shadow filesystem.");
        guard.volume.dismount()?;
        guard.volume.set_attributes(attributes)
    })();
    // Always release the successful lock and close/reopen the old handle,
    // even if dismount, the attribute change, or explicit unlocking fails.
    combine(operation, guard.finish(), "SNAPSHOT UNLOCK/REOPEN FAILED")
}

struct RestoreGuard<'a, V: Volume> {
    volume: &'a mut V,
    original: u64,
    armed: bool,
    remount: bool,
}

impl<V: Volume> RestoreGuard<'_, V> {
    fn restore(&mut self) -> Result<()> {
        self.armed = false;
        change_attributes(self.volume, self.original, self.remount)?;
        let observed = self.volume.attributes()?;
        if observed != self.original {
            return Err(format!(
                "Expected restored attributes {:#018x}, observed {observed:#018x}",
                self.original
            ));
        }
        if self.volume.is_writable()? {
            return Err(
                "Driver still reports writable after restoring read-only attributes".into(),
            );
        }
        println!("Remount completed; original driver attributes restored: {observed:#018x}. Filesystem integrity is not established.");
        Ok(())
    }
}

impl<V: Volume> Drop for RestoreGuard<'_, V> {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self.restore() {
                eprintln!("RESTORATION FAILED during unwinding: {error}");
                eprintln!(
                    "Use restore-readonly on this snapshot or revert the disposable VM checkpoint."
                );
            }
        }
    }
}

fn with_writable_snapshot<V: Volume>(
    volume: &mut V,
    remount: bool,
    action: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if !remount {
        return Err("Writable operations require remount-based transitions and restoration".into());
    }
    let original = volume.attributes()?;
    check_snapshot_attributes(original)?;
    if original & READ_ONLY == 0 {
        return Err(
            "Snapshot is already writable; refusing operation. Use restore-readonly if appropriate."
                .into(),
        );
    }
    if volume.is_writable()? {
        return Err("Writable status contradicts READ_ONLY; refusing mutation".into());
    }
    println!("Original attributes: {original:#018x}");
    let desired = original & !READ_ONLY;
    // Arm before the request: even a failed setter may have partially changed state.
    let mut guard = RestoreGuard {
        volume,
        original,
        armed: true,
        remount,
    };
    let operation = (|| {
        println!("Requesting temporary writable attributes: {desired:#018x}");
        change_attributes(guard.volume, desired, remount)
            .map_err(|error| format!("Write-enable request failed: {error}"))?;
        let observed = guard.volume.attributes()?;
        if observed != desired {
            return Err(format!(
                "Attribute transition not observed: expected {desired:#018x}, got {observed:#018x}"
            ));
        }
        if !guard.volume.is_writable()? {
            return Err("Driver still reports write protection after the attribute change".into());
        }
        println!("Driver reports writable; attempting the requested file operation.");
        action()
    })();
    let restoration = guard.restore();
    combine(
        operation,
        restoration,
        "RESTORATION FAILED; snapshot state is uncertain. Use restore-readonly or revert the VM checkpoint",
    )
}

fn restore_readonly<V: Volume>(volume: &mut V, remount: bool) -> Result<()> {
    let before = volume.attributes()?;
    check_snapshot_attributes(before)?;
    let desired = before | READ_ONLY;
    change_attributes(volume, desired, remount).map_err(|error| {
        format!("Read-only recovery failed; snapshot state is uncertain: {error}. Do not force dismount or use attribute-only recovery; revert the disposable VM checkpoint.")
    })?;
    let observed = volume.attributes()?;
    if observed != desired || volume.is_writable()? {
        return Err(format!(
            "Read-only restoration not confirmed; observed attributes {observed:#018x}"
        ));
    }
    println!("Remount completed; driver reports read-only; attributes: {observed:#018x}. Filesystem integrity is not established.");
    Ok(())
}

#[cfg(windows)]
fn run(options: Options) -> Result<()> {
    let path = snapshot_path(options.snapshot);
    println!("Target: {path}");
    let mut volume = windows::SnapshotVolume::open(&path)?;
    match options.command {
        Command::Inspect => {
            let attributes = volume.attributes()?;
            println!("GPT attributes: {attributes:#018x}");
            println!("SHADOW_COPY: {}", attributes & SHADOW_COPY != 0);
            println!("READ_ONLY: {}", attributes & READ_ONLY != 0);
            println!("Driver writable status: {}", volume.is_writable()?);
            Ok(())
        }
        Command::Probe => {
            with_writable_snapshot(&mut volume, options.remount, || {
                windows::marker_probe(&path)
            })?;
            println!(
                "Probe succeeded: test file verified and removed; original attributes restored."
            );
            Ok(())
        }
        Command::RestoreReadonly => restore_readonly(&mut volume, options.remount),
        Command::DiagnoseLock => {
            let relative = options
                .file
                .as_deref()
                .ok_or("diagnose-lock requires --file")?;
            diagnose_lock(&mut volume, || {
                deletion::preview(&path, relative).map(|_| ())
            })
        }
        Command::Delete => {
            check_snapshot_attributes(volume.attributes()?)?;
            let relative = options.file.as_deref().ok_or("delete requires --file")?;
            let expected = deletion::preview(&path, relative)?;
            if options.dry_run {
                println!("DRY RUN: no attributes changed and no file deleted.");
                return Ok(());
            }
            with_writable_snapshot(&mut volume, options.remount, || {
                deletion::delete_file(&path, relative, &expected)
            })?;
            println!("Selected file name deleted; original driver attributes restored. This does not restore file contents or filesystem metadata.");
            Ok(())
        }
    }
}

#[cfg(not(windows))]
fn run(_options: Options) -> Result<()> {
    Err("This experiment requires Windows. No operation was performed.".into())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("{error}\nUse --help for usage.");
            return ExitCode::from(2);
        }
    };
    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn probe<V: Volume>(
        volume: &mut V,
        remount: bool,
        action: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        with_writable_snapshot(volume, remount, action)
    }

    fn parse(arguments: &[&str]) -> Result<Option<Options>> {
        parse_args(
            &arguments
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn help_and_inspect_do_not_require_confirmation() {
        assert_eq!(parse(&[]).unwrap(), None);
        assert_eq!(parse(&["--help"]).unwrap(), None);
        assert_eq!(
            parse(&["inspect", "--snapshot", "42"]).unwrap(),
            Some(Options {
                command: Command::Inspect,
                snapshot: 42,
                remount: false,
                file: None,
                dry_run: false,
            })
        );
    }

    #[test]
    fn mutation_requires_confirmation() {
        for command in ["probe", "restore-readonly"] {
            assert!(parse(&[command, "--snapshot", "42"]).is_err());
            assert!(parse(&[command, "--confirm-lab", "--snapshot", "42"]).is_err());
            assert!(parse(&[
                command,
                "--confirm-lab",
                "--snapshot",
                "42",
                "--remount-snapshot"
            ])
            .is_ok());
        }
    }

    #[test]
    fn rejects_paths_invalid_numbers_and_duplicate_options() {
        for value in ["", "0", "-1", "+1", "4294967296", "C:", r"42\file", " 42"] {
            assert!(
                parse(&["inspect", "--snapshot", value]).is_err(),
                "{value:?}"
            );
        }
        assert!(parse(&["inspect", "--snapshot"]).is_err());
        assert!(parse(&["inspect"]).is_err());
        assert!(parse(&["inspect", "--snapshot", "1", "--snapshot", "2"]).is_err());
        assert!(parse(&["probe", "--snapshot", "1", "--confirm-lab", "--confirm-lab"]).is_err());
        assert!(parse(&["inspect", "--snapshot", "1", "--confirm-lab"]).is_err());
        assert!(parse(&["delete", "--snapshot", "1"]).is_err());
    }

    #[test]
    fn constructs_only_exact_shadow_paths() {
        assert_eq!(
            snapshot_path(42),
            r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy42"
        );
    }

    #[test]
    fn lock_diagnosis_requires_file_and_lab_confirmation_and_rejects_mutation_flags() {
        let base = [
            "diagnose-lock",
            "--snapshot",
            "3",
            "--file",
            r"Windows\Update\rec\data.bat",
            "--confirm-lab",
        ];
        assert_eq!(
            parse(&base).unwrap().unwrap().command,
            Command::DiagnoseLock
        );
        assert!(parse(&base[..5]).is_err());
        assert!(parse(&["diagnose-lock", "--snapshot", "3", "--confirm-lab"]).is_err());
        for flag in [
            "--dry-run",
            "--confirm-delete",
            "--remount-snapshot",
            "--restore-attributes-only",
        ] {
            let mut args = base.to_vec();
            args.push(flag);
            assert!(parse(&args).is_err(), "{flag}");
        }
    }

    #[test]
    fn lock_diagnosis_only_locks_unlocks_and_previews() {
        let mut volume = MockVolume::default();
        let mut previews = 0;
        diagnose_lock(&mut volume, || {
            previews += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(previews, 1);
        assert_eq!(volume.events, ["lock", "unlock", "lock", "unlock"]);
        assert!(volume.sets.is_empty());
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert!(!volume.locked);
    }

    #[test]
    fn lock_diagnosis_reports_each_failed_stage_without_force_or_retry() {
        for failures in [
            vec![("lock", 1)],
            vec![("lock", 2)],
            vec![("lock", 1), ("lock", 2)],
        ] {
            let mut volume = MockVolume {
                failures: failures.clone(),
                ..Default::default()
            };
            let mut previews = 0;
            let error = diagnose_lock(&mut volume, || {
                previews += 1;
                Ok(())
            })
            .unwrap_err();
            assert_eq!(previews, 1);
            assert_eq!(
                error.contains("BEFORE PREVIEW: LOCK FAILED"),
                failures.contains(&("lock", 1))
            );
            assert_eq!(
                error.contains("AFTER PREVIEW: LOCK FAILED"),
                failures.contains(&("lock", 2))
            );
            assert_eq!(
                volume
                    .events
                    .iter()
                    .filter(|&&event| event == "lock")
                    .count(),
                2
            );
            assert_eq!(
                volume
                    .events
                    .iter()
                    .filter(|&&event| event == "unlock")
                    .count(),
                2 - failures.len()
            );
            assert!(volume
                .events
                .iter()
                .all(|event| matches!(*event, "lock" | "unlock")));
            assert!(volume.sets.is_empty());
            assert!(!volume.locked);
        }
    }

    #[test]
    fn lock_diagnosis_stops_immediately_on_unlock_failure() {
        for failed_unlock in [1, 2] {
            let mut volume = MockVolume {
                failures: vec![("unlock", failed_unlock)],
                ..Default::default()
            };
            let mut previews = 0;
            let error = diagnose_lock(&mut volume, || {
                previews += 1;
                Ok(())
            })
            .unwrap_err();
            assert!(error.contains("UNLOCK FAILED"));
            assert_eq!(previews, failed_unlock - 1);
            assert_eq!(volume.events.len(), failed_unlock * 2);
            assert!(volume.sets.is_empty());
            // The real SnapshotVolume owns its handle, which closes on run's exit.
            assert!(volume.locked);
        }
    }

    #[test]
    fn lock_diagnosis_preserves_preview_and_prior_lock_errors() {
        for first_lock_fails in [false, true] {
            let mut volume = MockVolume::default();
            if first_lock_fails {
                volume.failures.push(("lock", 1));
            }
            let error = diagnose_lock(&mut volume, || Err("missing target".into())).unwrap_err();
            assert!(error.contains("PREVIEW FAILED: missing target"));
            assert_eq!(
                error.contains("BEFORE PREVIEW: LOCK FAILED"),
                first_lock_fails
            );
            assert!(!error.contains("AFTER PREVIEW"));
            assert!(volume.sets.is_empty());
            assert!(!volume.locked);
        }
    }

    #[test]
    fn lock_diagnosis_refuses_unexpected_or_writable_snapshots() {
        for (attributes, writable_override) in [
            (KNOWN_ATTRIBUTES & !READ_ONLY, None),
            (KNOWN_ATTRIBUTES, Some(true)),
            (KNOWN_ATTRIBUTES & !SHADOW_COPY, None),
        ] {
            let mut volume = MockVolume {
                attributes,
                writable_override,
                ..Default::default()
            };
            assert!(diagnose_lock(&mut volume, || panic!("preview must not run")).is_err());
            assert!(volume.events.is_empty());
            assert!(volume.sets.is_empty());
        }
    }

    struct MockVolume {
        attributes: u64,
        sets: Vec<u64>,
        fail_enable: bool,
        fail_restore: bool,
        ignore_enable: bool,
        deny_enabled_write: bool,
        writable_override: Option<bool>,
        events: Vec<&'static str>,
        failures: Vec<(&'static str, usize)>,
        panic_on: Option<&'static str>,
        locked: bool,
    }

    impl Default for MockVolume {
        fn default() -> Self {
            Self {
                attributes: KNOWN_ATTRIBUTES,
                sets: Vec::new(),
                fail_enable: false,
                fail_restore: false,
                ignore_enable: false,
                deny_enabled_write: false,
                writable_override: None,
                events: Vec::new(),
                failures: Vec::new(),
                panic_on: None,
                locked: false,
            }
        }
    }

    impl MockVolume {
        fn step(&mut self, name: &'static str) -> Result<()> {
            self.events.push(name);
            if self.panic_on == Some(name) {
                self.panic_on = None;
                panic!("simulated {name} panic");
            }
            let count = self.events.iter().filter(|event| **event == name).count();
            if self.failures.contains(&(name, count)) {
                Err(format!("simulated {name} failure"))
            } else {
                Ok(())
            }
        }
    }

    impl Volume for MockVolume {
        fn attributes(&mut self) -> Result<u64> {
            Ok(self.attributes)
        }

        fn set_attributes(&mut self, attributes: u64) -> Result<()> {
            self.step("set")?;
            self.sets.push(attributes);
            let enabling = attributes & READ_ONLY == 0;
            if !(enabling && self.ignore_enable) {
                self.attributes = attributes;
            }
            if enabling && self.fail_enable {
                Err("simulated partial enable failure".into())
            } else if !enabling && self.fail_restore {
                Err("simulated restoration failure".into())
            } else {
                Ok(())
            }
        }

        fn is_writable(&mut self) -> Result<bool> {
            if self.deny_enabled_write {
                return Ok(false);
            }
            Ok(self
                .writable_override
                .unwrap_or(self.attributes & READ_ONLY == 0))
        }

        fn lock(&mut self) -> Result<()> {
            self.step("lock")?;
            assert!(!self.locked);
            self.locked = true;
            Ok(())
        }

        fn dismount(&mut self) -> Result<()> {
            assert!(self.locked, "must lock successfully before dismounting");
            self.step("dismount")
        }

        fn unlock(&mut self) -> Result<()> {
            self.step("unlock")?;
            self.locked = false;
            Ok(())
        }

        fn reopen(&mut self) -> Result<()> {
            // Closing the old handle releases its lock even if explicit unlock failed.
            self.locked = false;
            self.step("reopen")
        }
    }

    #[test]
    fn successful_probe_restores_exact_original_attributes() {
        for original in [KNOWN_ATTRIBUTES, KNOWN_ATTRIBUTES & !HIDDEN] {
            let mut volume = MockVolume {
                attributes: original,
                ..Default::default()
            };
            let mut ran = false;
            probe(&mut volume, true, || {
                ran = true;
                Ok(())
            })
            .unwrap();
            assert!(ran);
            assert_eq!(volume.sets, [original & !READ_ONLY, original]);
            assert_eq!(volume.attributes, original);
        }
    }

    #[test]
    fn failed_file_operation_still_restores() {
        let mut volume = MockVolume::default();
        assert!(
            probe(&mut volume, true, || Err("file creation denied".into()))
                .unwrap_err()
                .contains("file creation denied")
        );
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert_eq!(volume.sets.len(), 2);
    }

    #[test]
    fn failed_enable_attempt_still_restores_without_running_action() {
        let mut volume = MockVolume {
            fail_enable: true,
            ..Default::default()
        };
        let error = probe(&mut volume, true, || panic!("must not run")).unwrap_err();
        assert!(error.contains("partial enable failure"));
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert_eq!(volume.sets.len(), 2);
    }

    #[test]
    fn preserves_both_operation_and_restoration_errors() {
        let mut volume = MockVolume {
            fail_restore: true,
            ..Default::default()
        };
        let error = probe(&mut volume, true, || Err("file failure".into())).unwrap_err();
        assert!(error.contains("file failure"));
        assert!(error.contains("RESTORATION FAILED"));
    }

    #[test]
    fn restoration_failure_prevents_success() {
        let mut volume = MockVolume {
            fail_restore: true,
            ..Default::default()
        };
        assert!(probe(&mut volume, true, || Ok(()))
            .unwrap_err()
            .contains("RESTORATION FAILED"));
    }

    #[test]
    fn rejects_unobserved_attribute_transition() {
        let mut volume = MockVolume {
            ignore_enable: true,
            ..Default::default()
        };
        assert!(probe(&mut volume, true, || panic!("must not run"))
            .unwrap_err()
            .contains("transition not observed"));
        assert_eq!(volume.sets.len(), 2);
    }

    #[test]
    fn refuses_write_probe_when_driver_still_reports_write_protection() {
        let mut volume = MockVolume {
            deny_enabled_write: true,
            ..Default::default()
        };
        assert!(probe(&mut volume, true, || panic!("must not run"))
            .unwrap_err()
            .contains("still reports write protection"));
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert_eq!(volume.sets.len(), 2);
    }

    #[test]
    fn refuses_already_writable_or_unexpected_attributes() {
        for attributes in [
            KNOWN_ATTRIBUTES & !READ_ONLY,
            READ_ONLY,
            KNOWN_ATTRIBUTES | 1,
        ] {
            let mut volume = MockVolume {
                attributes,
                ..Default::default()
            };
            assert!(probe(&mut volume, true, || panic!("must not run")).is_err());
            assert!(volume.sets.is_empty());
        }
    }

    #[test]
    fn refuses_inconsistent_initial_writable_status() {
        let mut volume = MockVolume {
            writable_override: Some(true),
            ..Default::default()
        };
        assert!(probe(&mut volume, true, || panic!("must not run")).is_err());
        assert!(volume.sets.is_empty());
    }

    #[test]
    fn unwinding_restores_attributes() {
        let mut volume = MockVolume::default();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = probe(&mut volume, true, || panic!("simulated panic"));
        }));
        assert!(result.is_err());
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert_eq!(volume.sets.len(), 2);
    }

    #[test]
    fn recovery_preserves_other_attributes() {
        let mut volume = MockVolume {
            attributes: KNOWN_ATTRIBUTES & !READ_ONLY,
            ..Default::default()
        };
        restore_readonly(&mut volume, true).unwrap();
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    }

    #[test]
    fn remount_is_explicit_and_not_allowed_on_inspect() {
        for command in ["probe", "restore-readonly"] {
            let args = [
                command,
                "--snapshot",
                "3",
                "--confirm-lab",
                "--remount-snapshot",
            ];
            assert!(parse(&args).unwrap().unwrap().remount);
            assert!(parse(&[command, "--snapshot", "3", "--remount-snapshot"]).is_err());
        }
        assert!(parse(&["inspect", "--snapshot", "3", "--remount-snapshot"]).is_err());
        assert!(parse(&[
            "probe",
            "--snapshot",
            "3",
            "--confirm-lab",
            "--remount-snapshot",
            "--remount-snapshot"
        ])
        .is_err());
    }

    #[test]
    fn remount_probe_runs_both_transitions_in_order() {
        let mut volume = MockVolume::default();
        probe(&mut volume, true, || Ok(())).unwrap();
        assert_eq!(
            volume.events,
            [
                "lock", "dismount", "set", "unlock", "reopen", "lock", "dismount", "set", "unlock",
                "reopen",
            ]
        );
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert!(!volume.locked);
    }

    #[test]
    fn lock_failure_never_forces_a_dismount_or_attribute_change() {
        let mut volume = MockVolume {
            failures: vec![("lock", 1)],
            ..Default::default()
        };
        assert!(change_attributes(&mut volume, KNOWN_ATTRIBUTES & !READ_ONLY, true).is_err());
        assert_eq!(volume.events, ["lock"]);
        assert!(volume.sets.is_empty());
        assert!(!volume.locked);
    }

    #[test]
    fn dismount_failure_unlocks_and_reopens_without_changing_attributes() {
        let mut volume = MockVolume {
            failures: vec![("dismount", 1)],
            ..Default::default()
        };
        assert!(change_attributes(&mut volume, KNOWN_ATTRIBUTES & !READ_ONLY, true).is_err());
        assert_eq!(volume.events, ["lock", "dismount", "unlock", "reopen"]);
        assert!(volume.sets.is_empty());
        assert!(!volume.locked);
    }

    #[test]
    fn each_failed_enable_stage_prevents_file_action_and_attempts_restoration() {
        for stage in ["lock", "dismount", "set", "unlock", "reopen"] {
            let mut volume = MockVolume {
                failures: vec![(stage, 1)],
                ..Default::default()
            };
            let error = probe(&mut volume, true, || panic!("must not run")).unwrap_err();
            assert!(error.contains(stage), "{error}");
            assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
            assert!(!volume.locked);
        }
    }

    #[test]
    fn partial_attribute_change_still_unlocks_reopens_and_restores() {
        let mut volume = MockVolume {
            fail_enable: true,
            ..Default::default()
        };
        assert!(probe(&mut volume, true, || panic!("must not run")).is_err());
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert_eq!(
            volume
                .events
                .iter()
                .filter(|name| **name == "reopen")
                .count(),
            2
        );
        assert!(!volume.locked);
    }

    #[test]
    fn restoration_remount_failure_is_not_success() {
        for stage in ["lock", "dismount", "set", "unlock", "reopen"] {
            let mut volume = MockVolume {
                failures: vec![(stage, 2)],
                ..Default::default()
            };
            let error = probe(&mut volume, true, || Ok(())).unwrap_err();
            assert!(error.contains("RESTORATION FAILED"), "{stage}: {error}");
            assert!(!volume.locked);
            if stage == "lock" {
                assert_eq!(volume.sets, [KNOWN_ATTRIBUTES & !READ_ONLY]);
                assert_eq!(
                    volume.events,
                    ["lock", "dismount", "set", "unlock", "reopen", "lock"]
                );
                assert_eq!(volume.attributes, KNOWN_ATTRIBUTES & !READ_ONLY);
            }
        }
    }

    #[test]
    fn remount_reports_primary_unlock_and_reopen_errors_together() {
        let mut volume = MockVolume {
            failures: vec![("dismount", 1), ("unlock", 1), ("reopen", 1)],
            ..Default::default()
        };
        let error =
            change_attributes(&mut volume, KNOWN_ATTRIBUTES & !READ_ONLY, true).unwrap_err();
        for expected in ["dismount", "unlock", "REOPEN FAILED"] {
            assert!(error.contains(expected), "{error}");
        }
        assert!(!volume.locked);
    }

    #[test]
    fn remount_restoration_also_runs_on_file_failure_and_unwinding() {
        let mut volume = MockVolume::default();
        assert!(probe(&mut volume, true, || Err("file is still read-only".into())).is_err());
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert_eq!(
            volume
                .events
                .iter()
                .filter(|name| **name == "reopen")
                .count(),
            2
        );
        let mut volume = MockVolume::default();
        assert!(catch_unwind(AssertUnwindSafe(|| {
            let _ = probe(&mut volume, true, || panic!("simulated file failure"));
        }))
        .is_err());
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert!(!volume.locked);
    }

    #[test]
    fn remount_recovery_preserves_other_attributes() {
        let mut volume = MockVolume {
            attributes: KNOWN_ATTRIBUTES & !READ_ONLY,
            ..Default::default()
        };
        restore_readonly(&mut volume, true).unwrap();
        assert_eq!(
            volume.events,
            ["lock", "dismount", "set", "unlock", "reopen"]
        );
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
    }

    #[test]
    fn unwinding_while_locked_releases_lock_before_restoring_attributes() {
        for stage in ["dismount", "set"] {
            let mut volume = MockVolume {
                panic_on: Some(stage),
                ..Default::default()
            };
            assert!(catch_unwind(AssertUnwindSafe(|| {
                let _ = probe(&mut volume, true, || panic!("must not run"));
            }))
            .is_err());
            assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
            assert!(!volume.locked);
            assert_eq!(
                volume
                    .events
                    .iter()
                    .filter(|name| **name == "reopen")
                    .count(),
                2
            );
        }
    }

    #[test]
    fn deletion_requires_file_and_two_confirmations_or_a_nonmutating_preview() {
        assert!(
            parse(&[
                "delete",
                "--snapshot",
                "3",
                "--file",
                r"Windows\test.txt",
                "--dry-run"
            ])
            .unwrap()
            .unwrap()
            .dry_run
        );
        assert!(parse(&[
            "delete",
            "--snapshot",
            "3",
            "--file",
            "test.txt",
            "--confirm-lab",
            "--confirm-delete",
            "--remount-snapshot"
        ])
        .is_ok());
        for flags in [
            vec![],
            vec!["--confirm-lab"],
            vec!["--confirm-delete"],
            vec!["--confirm-lab", "--confirm-delete"],
            vec!["--dry-run", "--confirm-delete"],
            vec!["--dry-run", "--remount-snapshot"],
        ] {
            let mut args = vec!["delete", "--snapshot", "3", "--file", "test.txt"];
            args.extend(flags);
            assert!(parse(&args).is_err());
        }
        assert!(parse(&[
            "delete",
            "--snapshot",
            "3",
            "--confirm-lab",
            "--confirm-delete"
        ])
        .is_err());
        assert!(parse(&[
            "probe",
            "--snapshot",
            "3",
            "--confirm-lab",
            "--file",
            "test.txt"
        ])
        .is_err());
        assert!(parse(&["inspect", "--snapshot", "3", "--dry-run"]).is_err());
    }

    #[test]
    fn rejects_unsafe_relative_file_paths() {
        for value in [
            "",
            r"\Windows\a",
            r"C:\a",
            r"..\a",
            r"a\..\b",
            r"a\.\b",
            r"\\server\a",
            r"a\\b",
            "a\\",
            "a/b",
            "a:stream",
            "*.bat",
            "a?.bat",
            "a\0b",
            "a\nb",
            "a.",
            "a ",
            r"a.\b",
            "NUL",
            "con.txt",
            r"a\COM1.log",
            "LPT9",
            "CONIN$",
            "CONOUT$",
            "con .txt",
            "COM\u{b9}.txt",
        ] {
            assert!(validate_relative_file(value).is_err(), "{value:?}");
        }
        for value in [
            r"Windows\Update\rec\data.bat",
            r"vss003-20230123T020915.9536740\Windows\Update\rec\data.bat",
            ".hidden",
            "file name.txt",
        ] {
            assert!(validate_relative_file(value).is_ok(), "{value}");
        }
    }

    #[test]
    fn removed_attribute_only_flag_is_rejected_with_an_explanation() {
        for command in [
            "probe",
            "delete",
            "restore-readonly",
            "inspect",
            "diagnose-lock",
        ] {
            for remount in [false, true] {
                let mut args = vec![command, "--snapshot", "3", "--restore-attributes-only"];
                if remount {
                    args.push("--remount-snapshot");
                }
                let error = parse(&args).unwrap_err();
                assert!(
                    error.contains("removed after NTFS flush failures"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn internal_entry_points_reject_attribute_only_transitions_without_mutation() {
        let mut volume = MockVolume::default();
        assert!(with_writable_snapshot(&mut volume, false, || panic!("must not run")).is_err());
        assert!(change_attributes(&mut volume, KNOWN_ATTRIBUTES, false).is_err());
        assert!(restore_readonly(&mut volume, false).is_err());
        assert_eq!(volume.attributes, KNOWN_ATTRIBUTES);
        assert!(volume.events.is_empty());
        assert!(volume.sets.is_empty());
    }
}
