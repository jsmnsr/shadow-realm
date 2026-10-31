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
