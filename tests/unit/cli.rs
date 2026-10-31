use super::*;

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
                    lab && !dry && !delete && !file && !source && !output
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
fn option_like_filenames_are_values_not_consent_flags() {
    for file in [
        "--",
        "-h",
        "--help",
        "--confirm-lab",
        "--dry-run",
        "--restore-attributes-only",
        "--debug",
        "debug",
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
fn help_and_debug_inspect_do_not_require_confirmation() {
    assert_eq!(parse(&[]).unwrap(), None);
    assert_eq!(parse(&["--help"]).unwrap(), None);
    assert_eq!(
        parse(&["debug", "inspect", "--snapshot", "42"]).unwrap(),
        Some(Options {
            command: Command::Inspect,
            snapshot: NonZeroU32::new(42).unwrap(),
            remount: false,
            file: None,
            source: None,
            output: None,
            dry_run: false,
        })
    );
}

#[test]
fn mutation_requires_confirmation() {
    for command in ["probe", "restore-readonly"] {
        assert!(parse(&[command, "--snapshot", "42"]).is_err());
        assert!(parse(&[command, "--confirm-lab", "--snapshot", "42"]).is_err());
        assert!(parse(&["debug", command, "--snapshot", "42"]).is_err());
        assert!(parse(&["debug", command, "--confirm-lab", "--snapshot", "42",]).is_ok());
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
        "debug",
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
fn remount_compatibility_is_debug_only_and_not_allowed_on_inspect() {
    for command in ["probe", "restore-readonly"] {
        let args = [
            "debug",
            command,
            "--snapshot",
            "3",
            "--confirm-lab",
            "--remount-snapshot",
        ];
        assert!(parse(&args).unwrap().unwrap().remount);
        assert!(parse(&[command, "--snapshot", "3", "--remount-snapshot"]).is_err());
    }
    assert!(parse(&["debug", "inspect", "--snapshot", "3", "--remount-snapshot"]).is_err());
    assert!(
        parse(&[
            "debug",
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
        ])
        .is_ok()
    );
    for flags in [
        vec![],
        vec!["--confirm-lab"],
        vec!["--confirm-delete"],
        vec!["--confirm-lab", "--confirm-delete", "--remount-snapshot"],
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
            "debug",
            "probe",
            "--snapshot",
            "3",
            "--confirm-lab",
            "--file",
            "test.txt"
        ])
        .is_err()
    );
    assert!(parse(&["debug", "inspect", "--snapshot", "3", "--dry-run"]).is_err());
}

#[test]
fn crud_flags_enforce_source_output_and_confirmation_boundaries() {
    for command in ["create", "update"] {
        let base = [
            command,
            "--snapshot",
            "3",
            "--file",
            r"dir\data.bin",
            "--source",
            "input.bin",
        ];
        assert!(parse(&base).is_err());
        for flag in ["--confirm-lab", "--dry-run"] {
            let mut args = base.to_vec();
            args.push(flag);
            let options = parse(&args).unwrap().unwrap();
            assert_eq!(options.remount, flag == "--confirm-lab");
            assert_eq!(options.dry_run, flag == "--dry-run");
            assert_eq!(options.source.as_deref(), Some("input.bin"));
            for extra in ["--confirm-delete", "--output", "--remount-snapshot", flag] {
                let mut invalid = args.clone();
                invalid.push(extra);
                assert!(parse(&invalid).is_err(), "{invalid:?}");
            }
        }
        assert!(parse(&base[..5]).is_err());
        let mut invalid = base.to_vec();
        invalid.extend(["--source", "second", "--confirm-lab"]);
        assert!(parse(&invalid).is_err());
        let mut invalid = base.to_vec();
        invalid.extend(["--dry-run", "--confirm-lab"]);
        assert!(parse(&invalid).is_err());
    }
    let read = ["read", "--snapshot", "3", "--file", "data.bin"];
    assert!(!parse(&read).unwrap().unwrap().remount);
    for flag in [
        "--confirm-lab",
        "--dry-run",
        "--confirm-delete",
        "--remount-snapshot",
        "--source",
    ] {
        let mut args = read.to_vec();
        args.push(flag);
        assert!(parse(&args).is_err());
    }
    let mut args = read.to_vec();
    args.extend(["--output", "export.bin"]);
    assert_eq!(
        parse(&args).unwrap().unwrap().output.as_deref(),
        Some("export.bin")
    );
    args.extend(["--output", "second"]);
    assert!(parse(&args).is_err());
    assert!(parse(&["read", "--snapshot", "3"]).is_err());
    for flag in ["--source", "--output"] {
        for value in ["", "--confirm-lab", "bad\0path"] {
            assert!(parse(&["read", "--snapshot", "3", "--file", "target", flag, value]).is_err());
        }
    }
}

#[test]
fn maintenance_commands_and_help_use_the_debug_namespace() {
    for command in ["inspect", "probe", "diagnose-lock", "restore-readonly"] {
        assert!(
            parse(&[command, "--snapshot", "3"])
                .unwrap_err()
                .to_string()
                .contains("debug subcommand")
        );
        assert!(!HELP.contains(&format!("shadow-realm {command} ")));
        assert!(DEBUG_HELP.contains(&format!("debug {command} ")));
    }
    for command in ["create", "read", "update", "delete"] {
        assert!(
            parse(&["debug", command, "--snapshot", "3"])
                .unwrap_err()
                .to_string()
                .contains("top-level")
        );
        assert!(!DEBUG_HELP.contains(&format!("shadow-realm {command} ")));
    }
    for args in [
        vec!["--debug"],
        vec!["--debug", "--help"],
        vec!["--help", "--debug"],
        vec!["--debug", "inspect", "--snapshot", "3"],
        vec!["inspect", "--snapshot", "3", "--debug"],
        vec!["debug", "inspect", "--snapshot", "3", "--debug"],
        vec!["debug", "debug", "inspect", "--snapshot", "3"],
        vec!["--help", "debug"],
        vec!["read", "--snapshot", "3", "--file", "target", "--debug"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
    for args in [vec!["debug"], vec!["debug", "--help"], vec!["debug", "-h"]] {
        assert_eq!(parse(&args).unwrap(), None);
    }
    assert!(!HELP.contains("--remount-snapshot"));
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
