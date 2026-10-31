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
#[path = "../tests/unit/cli.rs"]
mod tests;
