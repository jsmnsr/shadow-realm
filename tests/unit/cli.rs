use super::*;
use clap::error::ErrorKind;

fn parse(arguments: &[&str]) -> Result<Options, clap::Error> {
    parse_args(
        &arguments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
    )
}

#[test]
fn clap_command_tree_is_consistent() {
    command_tree().debug_assert();
}

#[test]
fn all_safety_flag_combinations_match_the_command_policy() {
    for command in [
        "inspect",
        "probe",
        "restore-readonly",
        "delete",
        "diagnose-lock",
        "create",
        "read",
        "update",
    ] {
        for mask in 0_u8..128 {
            let [lab, remount, dry, delete, file, source, output] =
                [0, 1, 2, 3, 4, 5, 6].map(|bit| mask & (1 << bit) != 0);
            let mut args = vec![command, "--snapshot", "3"];
            if matches!(
                command,
                "inspect" | "probe" | "restore-readonly" | "diagnose-lock"
            ) {
                args.insert(0, "debug");
            }
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
            if source {
                args.extend(["--source", "input.bin"]);
            }
            if output {
                args.extend(["--output", "output.bin"]);
            }
            let permitted = match command {
                "inspect" => mask == 0,
                "probe" | "restore-readonly" => {
                    lab && !remount && !dry && !delete && !file && !source && !output
                }
                "diagnose-lock" => file && lab && !remount && !dry && !delete && !source && !output,
                "delete" => {
                    file && !remount
                        && !source
                        && !output
                        && if dry { !lab && !delete } else { lab && delete }
                }
                "create" | "update" => {
                    file && source && !output && !remount && !delete && (lab != dry)
                }
                "read" => file && !lab && !remount && !dry && !delete && !source,
                _ => unreachable!(),
            };
            assert_eq!(parse(&args).is_ok(), permitted, "{args:?}");
        }
    }
}

#[test]
fn option_like_filenames_are_values_not_consent_or_help_flags() {
    for file in [
        "--",
        "-h",
        "--help",
        "--version",
        "--confirm-lab",
        "--dry-run",
        "--restore-attributes-only",
        "--debug",
        "debug",
    ] {
        let options = parse(&["delete", "--snapshot", "3", "--file", file, "--dry-run"]).unwrap();
        assert_eq!(options.file.as_deref(), Some(file));
        assert!(options.dry_run);
        assert_eq!(
            parse(&[
                "delete",
                "--snapshot=3",
                &format!("--file={file}"),
                "--dry-run"
            ])
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
    ] {
        let mut args = vec![
            "delete",
            "--snapshot",
            "3",
            "--file",
            "target",
            "--confirm-lab",
            "--confirm-delete",
        ];
        args.extend(extra);
        assert_eq!(
            parse(&args).unwrap_err().kind(),
            ErrorKind::ArgumentConflict,
            "{args:?}"
        );
    }
    for args in [
        vec![
            "delete",
            "--snapshot",
            "3",
            "--file",
            "target",
            "--dry-run",
            "--dry-run",
        ],
        vec![
            "create",
            "--snapshot",
            "3",
            "--file",
            "target",
            "--source",
            "first",
            "--source",
            "second",
            "--dry-run",
        ],
        vec![
            "read",
            "--snapshot",
            "3",
            "--file",
            "target",
            "--output",
            "first",
            "--output",
            "second",
        ],
    ] {
        assert_eq!(
            parse(&args).unwrap_err().kind(),
            ErrorKind::ArgumentConflict,
            "{args:?}"
        );
    }
}

#[test]
fn clap_handles_help_version_and_missing_commands() {
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["create", "--help"],
        vec!["read", "-h"],
        vec!["debug", "--help"],
        vec!["debug", "-h"],
        vec!["help", "debug", "inspect"],
    ] {
        assert_eq!(
            parse(&args).unwrap_err().kind(),
            ErrorKind::DisplayHelp,
            "{args:?}"
        );
    }
    assert_eq!(
        parse(&["--version"]).unwrap_err().kind(),
        ErrorKind::DisplayVersion
    );
    for args in [vec![], vec!["debug"]] {
        let error = parse(&args).unwrap_err();
        assert_eq!(
            error.kind(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        assert_eq!(error.exit_code(), 2);
    }
    assert_eq!(
        parse(&["debug", "inspect", "--snapshot", "42"]).unwrap(),
        Options {
            command: Command::Inspect,
            snapshot: NonZeroU32::new(42).unwrap(),
            file: None,
            source: None,
            output: None,
            dry_run: false,
        }
    );
}

#[test]
fn typed_snapshot_and_path_validators_run_inside_clap() {
    for value in ["", "0", "-1", "+1", "4294967296", "C:", r"42\file", " 42"] {
        assert!(
            parse(&["debug", "inspect", "--snapshot", value]).is_err(),
            "{value:?}"
        );
        let error = parse(&["debug", "inspect", &format!("--snapshot={value}")]).unwrap_err();
        assert_eq!(
            error.kind(),
            ErrorKind::ValueValidation,
            "{value:?}: {error}"
        );
    }
    for value in [
        "",
        r"..\outside",
        r"C:\source",
        r"dir\NUL",
        "*.txt",
        "bad\0path",
    ] {
        assert!(
            parse(&["read", "--snapshot", "3", "--file", value]).is_err(),
            "{value:?}"
        );
    }
    for value in ["", "--confirm-lab", "bad\0path"] {
        assert!(
            parse(&[
                "create",
                "--snapshot",
                "3",
                "--file",
                "target",
                &format!("--source={value}"),
                "--dry-run"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "read",
                "--snapshot",
                "3",
                "--file",
                "target",
                &format!("--output={value}")
            ])
            .is_err()
        );
    }
}

#[test]
fn mutations_require_consent_but_never_an_explicit_remount_flag() {
    for (name, command) in [("create", Command::Create), ("update", Command::Update)] {
        for flag in ["--confirm-lab", "--dry-run"] {
            let options = parse(&[
                name,
                "--snapshot=3",
                r"--file=dir\data.bin",
                "--source=input.bin",
                flag,
            ])
            .unwrap();
            assert_eq!(options.command, command);
            assert_eq!(options.source.as_deref(), Some("input.bin"));
            assert_eq!(options.dry_run, flag == "--dry-run");
        }
    }
    for (name, command) in [
        ("probe", Command::Probe),
        ("restore-readonly", Command::RestoreReadonly),
    ] {
        assert_eq!(
            parse(&["debug", name, "--snapshot", "3"])
                .unwrap_err()
                .kind(),
            ErrorKind::MissingRequiredArgument
        );
        let options = parse(&["debug", name, "--snapshot", "3", "--confirm-lab"]).unwrap();
        assert_eq!(options.command, command);
    }
    let options = parse(&[
        "delete",
        "--snapshot",
        "3",
        "--file",
        "target",
        "--confirm-lab",
        "--confirm-delete",
    ])
    .unwrap();
    assert_eq!(options.command, Command::Delete);
    let diagnosis = parse(&[
        "debug",
        "diagnose-lock",
        "--snapshot",
        "3",
        "--file",
        "target",
        "--confirm-lab",
    ])
    .unwrap();
    assert_eq!(diagnosis.command, Command::DiagnoseLock);
    let read = parse(&[
        "read",
        "--snapshot",
        "3",
        "--file",
        "target",
        "--output",
        "export.bin",
    ])
    .unwrap();
    assert_eq!(read.command, Command::Read);
    assert_eq!(read.output.as_deref(), Some("export.bin"));
}

#[test]
fn clap_rejects_commands_outside_their_namespace() {
    for command in ["inspect", "probe", "diagnose-lock", "restore-readonly"] {
        assert_eq!(
            parse(&[command, "--snapshot", "3"]).unwrap_err().kind(),
            ErrorKind::InvalidSubcommand
        );
    }
    for command in ["create", "read", "update", "delete"] {
        assert_eq!(
            parse(&["debug", command, "--snapshot", "3"])
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidSubcommand
        );
    }
    for args in [
        vec!["--debug", "inspect", "--snapshot", "3"],
        vec!["debug", "inspect", "--snapshot", "3", "--debug"],
        vec!["read", "--snapshot", "3", "--file", "target", "--debug"],
        vec!["--snapshot", "3", "read"],
    ] {
        assert_eq!(
            parse(&args).unwrap_err().kind(),
            ErrorKind::UnknownArgument,
            "{args:?}"
        );
    }
}

#[test]
fn removed_transition_flags_are_not_accepted_by_any_command() {
    for command in [
        "create",
        "read",
        "update",
        "delete",
        "inspect",
        "probe",
        "restore-readonly",
        "diagnose-lock",
    ] {
        for flag in ["--restore-attributes-only", "--remount-snapshot"] {
            let mut args = vec![command, "--snapshot", "3", flag];
            if matches!(
                command,
                "inspect" | "probe" | "restore-readonly" | "diagnose-lock"
            ) {
                args.insert(0, "debug");
            }
            let error = parse(&args).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::UnknownArgument, "{args:?}");
            assert!(error.to_string().contains(flag));
        }
    }
}
