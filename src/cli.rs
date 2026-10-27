//! Command-line parsing and consent checks; never opens files or devices.

#![forbid(unsafe_code)]

use crate::paths::validate_relative_file;
use anyhow::{Context, Result};
use std::num::NonZeroU32;

pub(crate) const HELP: &str = concat!(
    "\
Shadow Realm ",
    env!("CARGO_PKG_VERSION"),
    " -- DISPOSABLE WINDOWS VM ONLY

  shadow-realm inspect --snapshot NUMBER
  shadow-realm diagnose-lock --snapshot NUMBER --file RELATIVE_PATH --confirm-lab
  shadow-realm probe --snapshot NUMBER --confirm-lab --remount-snapshot
  shadow-realm restore-readonly --snapshot NUMBER --confirm-lab --remount-snapshot
  shadow-realm delete --snapshot NUMBER --file RELATIVE_PATH --dry-run
  shadow-realm delete --snapshot NUMBER --file RELATIVE_PATH --confirm-lab --confirm-delete
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
pub(crate) enum Command {
    Inspect,
    Probe,
    RestoreReadonly,
    Delete,
    DiagnoseLock,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub(crate) command: Command,
    pub(crate) snapshot: NonZeroU32,
    pub(crate) remount: bool,
    pub(crate) file: Option<String>,
    pub(crate) dry_run: bool,
}

pub(crate) fn parse_args(args: &[String]) -> Result<Option<Options>> {
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        return Ok(None);
    }
    let command = match args[0].as_str() {
        "inspect" => Command::Inspect,
        "probe" => Command::Probe,
        "restore-readonly" => Command::RestoreReadonly,
        "delete" => Command::Delete,
        "diagnose-lock" => Command::DiagnoseLock,
        other => anyhow::bail!("Unknown command: {other}"),
    };
    let mut matches = clap::Command::new("shadow-realm")
        .no_binary_name(true)
        .disable_help_flag(true)
        .args([
            clap::arg!(--snapshot <NUMBER>),
            clap::arg!(--file <RELATIVE_PATH>).allow_hyphen_values(true),
            clap::arg!(--"confirm-lab"),
            clap::arg!(--"remount-snapshot"),
            clap::arg!(--"dry-run"),
            clap::arg!(--"confirm-delete"),
            clap::arg!(--"restore-attributes-only"),
        ])
        .try_get_matches_from(&args[1..])?;
    if matches.get_flag("restore-attributes-only") {
        anyhow::bail!(
            "--restore-attributes-only was removed after NTFS flush failures. Attribute flags alone do not establish clean filesystem recovery. Use --remount-snapshot; if restoration fails, revert the disposable VM checkpoint."
        );
    }
    let value = matches
        .get_one::<String>("snapshot")
        .context("--snapshot is required")?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        anyhow::bail!("--snapshot must be a positive decimal number, not a path");
    }
    let number = value
        .parse::<u32>()
        .context("--snapshot is outside the u32 range")?;
    let snapshot = NonZeroU32::new(number).context("--snapshot must be greater than zero")?;
    let confirmed = matches.get_flag("confirm-lab");
    let remount = matches.get_flag("remount-snapshot");
    let dry_run = matches.get_flag("dry-run");
    let confirm_delete = matches.get_flag("confirm-delete");
    let file = matches.remove_one::<String>("file");
    if let Some(value) = &file {
        validate_relative_file(value)?;
    }
    if command == Command::Delete {
        if file.is_none() {
            anyhow::bail!("delete requires --file with a snapshot-relative path");
        }
        if dry_run && (confirmed || confirm_delete || remount) {
            anyhow::bail!("--dry-run cannot be combined with mutation or confirmation flags");
        }
        if !dry_run && !confirm_delete {
            anyhow::bail!(
                "Deletion requires --confirm-delete; use --dry-run to preview the target"
            );
        }
    } else if command == Command::DiagnoseLock {
        if file.is_none() {
            anyhow::bail!("diagnose-lock requires --file with a snapshot-relative path");
        }
        if dry_run || confirm_delete || remount {
            anyhow::bail!("diagnose-lock does not accept deletion, dry-run, or remount flags");
        }
    } else if file.is_some() || dry_run || confirm_delete {
        anyhow::bail!("--file requires delete/diagnose-lock; deletion flags require delete");
    }
    if matches!(
        command,
        Command::Probe | Command::RestoreReadonly | Command::Delete
    ) && !dry_run
        && !remount
    {
        anyhow::bail!(
            "Mutations require --remount-snapshot. Attribute-only transitions are no longer supported after lab NTFS flush failures."
        );
    }
    if command != Command::Inspect && !dry_run && !confirmed {
        anyhow::bail!("Mutation and locking commands require --confirm-lab; use a disposable VM");
    }
    if command == Command::Inspect && (confirmed || remount) {
        anyhow::bail!("inspect does not use --confirm-lab or --remount-snapshot");
    }
    Ok(Some(Options {
        command,
        snapshot,
        remount,
        file,
        dry_run,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_safety_flag_combinations_match_the_command_policy() {
        for command in [
            "inspect",
            "probe",
            "restore-readonly",
            "delete",
            "diagnose-lock",
        ] {
            for mask in 0_u8..32 {
                let [lab, remount, dry, delete, file] =
                    [0, 1, 2, 3, 4].map(|bit| mask & (1 << bit) != 0);
                let mut args = vec![command, "--snapshot", "3"];
                for (present, flag) in [
                    (lab, "--confirm-lab"),
                    (remount, "--remount-snapshot"),
                    (dry, "--dry-run"),
                    (delete, "--confirm-delete"),
                ] {
                    if present {
                        args.push(flag);
                    }
                }
                if file {
                    args.extend(["--file", "target"]);
                }
                let permitted = match command {
                    "inspect" => mask == 0,
                    "probe" | "restore-readonly" => lab && remount && !dry && !delete && !file,
                    "diagnose-lock" => file && lab && !remount && !dry && !delete,
                    "delete" => {
                        file && if dry {
                            !lab && !remount && !delete
                        } else {
                            lab && remount && delete
                        }
                    }
                    _ => unreachable!(),
                };
                assert_eq!(parse(&args).is_ok(), permitted, "{args:?}");
            }
        }
    }

    #[test]
    fn option_like_filenames_are_values_not_consent_flags() {
        for file in [
            "--",
            "-h",
            "--help",
            "--confirm-lab",
            "--dry-run",
            "--restore-attributes-only",
        ] {
            let options = parse(&["delete", "--snapshot", "3", "--file", file, "--dry-run"])
                .unwrap()
                .unwrap();
            assert_eq!(options.file.as_deref(), Some(file));
            assert!(options.dry_run);
            assert_eq!(
                parse(&[
                    "delete",
                    "--snapshot=3",
                    &format!("--file={file}"),
                    "--dry-run"
                ])
                .unwrap()
                .unwrap(),
                options
            );
        }
    }

    #[test]
    fn duplicate_values_and_switches_are_never_overridden() {
        for extra in [
            vec!["--snapshot", "4"],
            vec!["--snapshot=4"],
            vec!["--file", "other"],
            vec!["--file=other"],
            vec!["--confirm-lab"],
            vec!["--confirm-delete"],
            vec!["--remount-snapshot"],
        ] {
            let mut args = vec![
                "delete",
                "--snapshot",
                "3",
                "--file",
                "target",
                "--confirm-lab",
                "--confirm-delete",
                "--remount-snapshot",
            ];
            args.extend(extra);
            assert!(parse(&args).is_err(), "{args:?}");
        }
        assert!(
            parse(&[
                "delete",
                "--snapshot",
                "3",
                "--file",
                "target",
                "--dry-run",
                "--dry-run"
            ])
            .is_err()
        );
        assert!(parse(&["--snapshot", "3", "inspect"]).is_err());
    }

    fn parse(arguments: &[&str]) -> Result<Option<Options>> {
        parse_args(
            &arguments
                .iter()
                .map(ToString::to_string)
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
                snapshot: NonZeroU32::new(42).unwrap(),
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
            assert!(
                parse(&[
                    command,
                    "--confirm-lab",
                    "--snapshot",
                    "42",
                    "--remount-snapshot"
                ])
                .is_ok()
            );
        }
    }

    #[test]
    fn rejects_paths_invalid_numbers_and_duplicate_options() {
        for value in ["", "0", "-1", "+1", "4294967296", "C:", r"42\file", " 42"] {
            assert!(
                parse(&["inspect", "--snapshot", value]).is_err(),
                "{value:?}"
            );
            assert!(
                parse(&["inspect", &format!("--snapshot={value}")]).is_err(),
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
        assert!(
            parse(&[
                "probe",
                "--snapshot",
                "3",
                "--confirm-lab",
                "--remount-snapshot",
                "--remount-snapshot"
            ])
            .is_err()
        );
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
        assert!(
            parse(&[
                "delete",
                "--snapshot",
                "3",
                "--file",
                "test.txt",
                "--confirm-lab",
                "--confirm-delete",
                "--remount-snapshot"
            ])
            .is_ok()
        );
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
        assert!(
            parse(&[
                "delete",
                "--snapshot",
                "3",
                "--confirm-lab",
                "--confirm-delete"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "probe",
                "--snapshot",
                "3",
                "--confirm-lab",
                "--file",
                "test.txt"
            ])
            .is_err()
        );
        assert!(parse(&["inspect", "--snapshot", "3", "--dry-run"]).is_err());
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
                    format!("{error:#}").contains("removed after NTFS flush failures"),
                    "{error:#}"
                );
            }
        }
    }
}
