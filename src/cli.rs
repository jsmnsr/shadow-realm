//! Clap command definitions, generated help, and typed argument validation.

#![forbid(unsafe_code)]

use crate::paths::validate_relative_file;
use clap::{Arg, ArgAction, ArgMatches, Command as ClapCommand};
use std::num::NonZeroU32;

const SAFETY_NOTICE: &str = "\
DISPOSABLE WINDOWS VM ONLY. Run elevated; never use production snapshots.
Snapshot numbers are HarddiskVolumeShadowCopy suffixes, not VSS GUIDs.
Mutations automatically remount writable, operate, then restore read-only.
Busy snapshots are never force-dismounted. Attribute-only transitions are disabled.
Restoration is best-effort, not rollback or proof of filesystem integrity.
Errors, termination, power loss, or driver failures can leave uncertain state.";

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
    pub(crate) file: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) output: Option<String>,
    pub(crate) dry_run: bool,
}

fn command_tree() -> ClapCommand {
    ClapCommand::new("shadow-realm")
        .bin_name("shadow-realm")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Create, read, update, and delete files in a selected VSS shadow copy")
        .after_help(SAFETY_NOTICE)
        .no_binary_name(true)
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            mutation(
                "create",
                "Create a new file without overwriting an existing name",
            )
            .arg(source_arg()),
        )
        .subcommand(
            leaf("read", "Read exact file bytes without remounting")
                .arg(file_arg())
                .arg(
                    local_path_arg("output").help(
                        "Create a new local output file; otherwise write raw bytes to stdout",
                    ),
                ),
        )
        .subcommand(
            mutation("update", "Replace an existing file's entire contents")
                .long_about(
                    "Replace an existing file's entire contents, including truncation. \
                     Updates are not atomic and reject files with multiple hard links.",
                )
                .arg(source_arg()),
        )
        .subcommand(
            mutation("delete", "Delete exactly one existing regular-file name").arg(
                flag("confirm-delete", "Confirm deletion of the selected name")
                    .required_unless_present("dry-run")
                    .conflicts_with("dry-run"),
            ),
        )
        .subcommand(
            ClapCommand::new("debug")
                .about("Low-level inspection, diagnosis, probing, and recovery")
                .after_help(SAFETY_NOTICE)
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(leaf(
                    "inspect",
                    "Query attributes and driver writable status",
                ))
                .subcommand(
                    leaf(
                        "diagnose-lock",
                        "Compare volume locking before and after file preview",
                    )
                    .long_about(
                        "Compare lock/unlock before and after previewing an existing file. \
                             No attribute changes or dismounts; locking can flush caches and \
                             briefly exclude other access.",
                    )
                    .arg(file_arg())
                    .arg(lab_consent().required(true)),
                )
                .subcommand(
                    leaf(
                        "probe",
                        "Create, verify, and remove an owned test file, then restore read-only",
                    )
                    .arg(lab_consent().required(true)),
                )
                .subcommand(
                    leaf(
                        "restore-readonly",
                        "Best-effort read-only recovery, preserving other attributes; not undo",
                    )
                    .arg(lab_consent().required(true)),
                ),
        )
}

fn leaf(name: &'static str, about: &'static str) -> ClapCommand {
    ClapCommand::new(name)
        .about(about)
        .after_help(SAFETY_NOTICE)
        .arg(
            Arg::new("snapshot")
                .long("snapshot")
                .value_name("NUMBER")
                .help("Positive numeric suffix of the selected shadow device")
                .required(true)
                .value_parser(snapshot_number),
        )
}

fn mutation(name: &'static str, about: &'static str) -> ClapCommand {
    leaf(name, about)
        .arg(file_arg())
        .arg(
            lab_consent()
                .required_unless_present("dry-run")
                .conflicts_with("dry-run"),
        )
        .arg(flag(
            "dry-run",
            "Preview without changing attributes or file contents",
        ))
}

fn flag(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .action(ArgAction::SetTrue)
        .help(help)
}

fn lab_consent() -> Arg {
    flag(
        "confirm-lab",
        "Confirm this is a disposable Windows VM with sacrificial data",
    )
}

fn file_arg() -> Arg {
    Arg::new("file")
        .long("file")
        .value_name("RELATIVE_PATH")
        .help(
            "One snapshot-relative regular file; existing parents, no wildcards or reparse points",
        )
        .required(true)
        .allow_hyphen_values(true)
        .value_parser(relative_file)
}

fn local_path_arg(name: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .value_name("LOCAL_FILE")
        .value_parser(local_path)
}

fn source_arg() -> Arg {
    local_path_arg("source")
        .help("Local regular file; exact bytes buffered in memory before remounting")
        .required(true)
}

fn snapshot_number(value: &str) -> Result<NonZeroU32, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("must be a positive decimal number, not a path".into());
    }
    let number = value
        .parse::<u32>()
        .map_err(|error| format!("--snapshot is outside the u32 range: {error}"))?;
    NonZeroU32::new(number).ok_or_else(|| "--snapshot must be greater than zero".into())
}

fn relative_file(value: &str) -> Result<String, String> {
    validate_relative_file(value).map_err(|error| error.to_string())?;
    Ok(value.to_owned())
}

fn local_path(value: &str) -> Result<String, String> {
    if value.is_empty() || value.starts_with("--") || value.contains('\0') {
        return Err("must be a nonempty local path (prefix option-like names with .\\)".into());
    }
    Ok(value.to_owned())
}

pub(crate) fn parse_args(args: &[String]) -> Result<Options, clap::Error> {
    let matches = command_tree().try_get_matches_from(args)?;
    let (command, matches) = match matches.subcommand() {
        Some(("create", matches)) => (Command::Create, matches),
        Some(("read", matches)) => (Command::Read, matches),
        Some(("update", matches)) => (Command::Update, matches),
        Some(("delete", matches)) => (Command::Delete, matches),
        Some(("debug", matches)) => match matches.subcommand() {
            Some(("inspect", matches)) => (Command::Inspect, matches),
            Some(("diagnose-lock", matches)) => (Command::DiagnoseLock, matches),
            Some(("probe", matches)) => (Command::Probe, matches),
            Some(("restore-readonly", matches)) => (Command::RestoreReadonly, matches),
            _ => unreachable!("Clap requires a declared debug subcommand"),
        },
        _ => unreachable!("Clap requires a declared subcommand"),
    };
    Ok(options_from_matches(command, matches))
}

fn options_from_matches(command: Command, matches: &ArgMatches) -> Options {
    let mutation = matches!(command, Command::Create | Command::Update | Command::Delete);
    let dry_run = mutation && matches.get_flag("dry-run");
    Options {
        command,
        snapshot: *matches
            .get_one::<NonZeroU32>("snapshot")
            .expect("Clap requires a validated snapshot number"),
        file: if mutation || matches!(command, Command::Read | Command::DiagnoseLock) {
            matches.get_one::<String>("file").cloned()
        } else {
            None
        },
        source: if matches!(command, Command::Create | Command::Update) {
            matches.get_one::<String>("source").cloned()
        } else {
            None
        },
        output: if command == Command::Read {
            matches.get_one::<String>("output").cloned()
        } else {
            None
        },
        dry_run,
    }
}

#[cfg(test)]
#[path = "../tests/unit/cli.rs"]
mod tests;
