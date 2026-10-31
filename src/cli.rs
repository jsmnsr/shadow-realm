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

  shadow-realm create --snapshot NUMBER --file RELATIVE_PATH --source LOCAL_FILE --confirm-lab
  shadow-realm read --snapshot NUMBER --file RELATIVE_PATH [--output LOCAL_FILE]
  shadow-realm update --snapshot NUMBER --file RELATIVE_PATH --source LOCAL_FILE --confirm-lab
  shadow-realm delete --snapshot NUMBER --file RELATIVE_PATH --dry-run
  shadow-realm delete --snapshot NUMBER --file RELATIVE_PATH --confirm-lab --confirm-delete

create: create one new regular file; existing files are never overwritten.
read: copy exact bytes to stdout, or create a new --output file. No remount.
update: replace an existing regular file's contents with bytes from --source.
delete: remove exactly one existing regular-file name. Requires --confirm-delete.
--source: local regular file, buffered in memory before any remount.
--dry-run: preview create/update/delete without changing the snapshot; no confirmations.
Parents must already exist. No wildcards, traversal, reparse points, or permission bypass.
Updates reject multiple hard links to avoid changing other names.
Mutations automatically lock/remount writable, operate, then remount read-only.
Busy snapshots are never force-dismounted. Restoration is not rollback.
Use debug --help for low-level inspection, diagnosis, probing, and recovery.

NUMBER is the numeric suffix of HarddiskVolumeShadowCopyNUMBER, NOT a VSS GUID.
Only exact shadow-device targets are accepted; source disks and paths are not.
Being the newest snapshot is NOT sufficient to guarantee write support.
Run elevated. Do not use production snapshots or host-mounted production disks.
Driver read-only status does NOT prove filesystem integrity or full recovery.
Restoration is best-effort on failure and is NOT guaranteed after termination,
power loss, a crash, or a driver error. File cleanup is not a full rollback.
"
);

pub(crate) const DEBUG_HELP: &str = "\
Shadow Realm debug commands -- DISPOSABLE WINDOWS VM ONLY

  shadow-realm debug inspect --snapshot NUMBER
  shadow-realm debug diagnose-lock --snapshot NUMBER --file RELATIVE_PATH --confirm-lab
  shadow-realm debug probe --snapshot NUMBER --confirm-lab
  shadow-realm debug restore-readonly --snapshot NUMBER --confirm-lab

inspect: query attributes and the driver's writable status; no mutations.
diagnose-lock: compare lock/unlock before and after previewing an existing file.
               No attribute changes, dismounts, or deletion. Locking can flush
               filesystem caches and briefly excludes other access.
probe: temporarily request writable attributes, create/read/delete one unique
       test file in the snapshot root, then restore the original attributes.
restore-readonly: recovery action; set READ_ONLY while preserving other attributes.
--remount-snapshot: debug-only compatibility flag; mutations already remount.
Attribute-only restoration was removed after lab NTFS flush failures.
Busy snapshots are never force-dismounted. Restoration is best-effort, not rollback.
Run elevated in a disposable VM; never use production snapshots.
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Inspect,
    Probe,
    RestoreReadonly,
    Create,
    Read,
    Update,
    Delete,
    DiagnoseLock,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub(crate) command: Command,
    pub(crate) snapshot: NonZeroU32,
    pub(crate) remount: bool,
    pub(crate) file: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) output: Option<String>,
    pub(crate) dry_run: bool,
}

pub(crate) fn parse_args(args: &[String]) -> Result<Option<Options>> {
    if args.is_empty()
        || args == ["--help"]
        || args == ["-h"]
        || args == ["debug"]
        || args == ["debug", "--help"]
        || args == ["debug", "-h"]
    {
        return Ok(None);
    }
    let debug = args[0] == "debug";
    let command_index = usize::from(debug);
    let command = match args[command_index].as_str() {
        "inspect" => Command::Inspect,
        "probe" => Command::Probe,
        "restore-readonly" => Command::RestoreReadonly,
        "create" => Command::Create,
        "read" => Command::Read,
        "update" => Command::Update,
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
            clap::arg!(--source <LOCAL_FILE>),
            clap::arg!(--output <LOCAL_FILE>),
            clap::arg!(--"confirm-lab"),
            clap::arg!(--"remount-snapshot"),
            clap::arg!(--"dry-run"),
            clap::arg!(--"confirm-delete"),
            clap::arg!(--"restore-attributes-only"),
        ])
        .try_get_matches_from(&args[command_index + 1..])?;
    if matches.get_flag("restore-attributes-only") {
        anyhow::bail!(
            "--restore-attributes-only was removed after NTFS flush failures. Attribute flags alone do not establish clean filesystem recovery. Mutations now remount automatically; if restoration fails, revert the disposable VM checkpoint."
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
    let maintenance = matches!(
        command,
        Command::Inspect | Command::Probe | Command::RestoreReadonly | Command::DiagnoseLock
    );
    if maintenance && !debug {
        anyhow::bail!("This maintenance command requires the debug subcommand; use debug --help");
    }
    if debug && !maintenance {
        anyhow::bail!("CRUD commands are top-level; do not prefix them with debug");
    }
    if remount && !debug {
        anyhow::bail!(
            "--remount-snapshot is debug-only and no longer needed; mutations remount automatically"
        );
    }
    let options = Options {
        command,
        snapshot,
        remount: !dry_run
            && matches!(
                command,
                Command::Create
                    | Command::Update
                    | Command::Delete
                    | Command::Probe
                    | Command::RestoreReadonly
            ),
        file,
        source: matches.remove_one::<String>("source"),
        output: matches.remove_one::<String>("output"),
        dry_run,
    };
    validate_options(&options, confirmed, confirm_delete, remount)?;
    Ok(Some(options))
}

fn validate_options(
    options: &Options,
    confirmed: bool,
    confirm_delete: bool,
    remount: bool,
) -> Result<()> {
    let command = options.command;
    for (flag, value) in [("--source", &options.source), ("--output", &options.output)] {
        if value.as_ref().is_some_and(|value| {
            value.is_empty() || value.starts_with("--") || value.contains('\0')
        }) {
            anyhow::bail!("{flag} requires a nonempty local path");
        }
    }
    if matches!(command, Command::Create | Command::Update | Command::Delete) {
        if options.file.is_none() {
            anyhow::bail!("create/update/delete require --file with a snapshot-relative path");
        }
        if options.dry_run && (confirmed || confirm_delete || remount) {
            anyhow::bail!("--dry-run cannot be combined with mutation or confirmation flags");
        }
        if command == Command::Delete && !options.dry_run && !confirm_delete {
            anyhow::bail!(
                "Deletion requires --confirm-delete; use --dry-run to preview the target"
            );
        }
    } else if matches!(command, Command::Read | Command::DiagnoseLock) {
        if options.file.is_none() {
            anyhow::bail!("read/diagnose-lock require --file with a snapshot-relative path");
        }
        if options.dry_run || confirm_delete || remount {
            anyhow::bail!("read/diagnose-lock do not accept deletion, dry-run, or remount flags");
        }
    } else if options.file.is_some() || options.dry_run || confirm_delete {
        anyhow::bail!(
            "--file requires a file operation or diagnose-lock; --dry-run requires a mutation"
        );
    }
    if confirm_delete && command != Command::Delete {
        anyhow::bail!("--confirm-delete requires delete");
    }
    if matches!(command, Command::Create | Command::Update) {
        if options.source.is_none() {
            anyhow::bail!("create/update require --source with a local regular file");
        }
    } else if options.source.is_some() {
        anyhow::bail!("--source requires create or update");
    }
    if options.output.is_some() && command != Command::Read {
        anyhow::bail!("--output requires read");
    }
    if !matches!(command, Command::Inspect | Command::Read) && !options.dry_run && !confirmed {
        anyhow::bail!("Mutation and locking commands require --confirm-lab; use a disposable VM");
    }
    if matches!(command, Command::Inspect | Command::Read) && (confirmed || remount) {
        anyhow::bail!("inspect/read do not use --confirm-lab or --remount-snapshot");
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/cli.rs"]
mod tests;
