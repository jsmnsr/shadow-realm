//! Black-box CLI checks that never request a valid shadow-device operation.

#![forbid(unsafe_code)]

use std::process::{Command, Output};

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_shadow-realm"))
        .args(arguments)
        .output()
        .expect("built Shadow Realm executable must start")
}

fn has_command(help: &str, command: &str) -> bool {
    help.lines()
        .any(|line| line.starts_with("  ") && line.split_whitespace().next() == Some(command))
}

#[test]
fn generated_help_lists_only_commands_in_the_selected_namespace() {
    for (args, visible, hidden) in [
        (
            vec!["--help"],
            ["create", "read", "update", "delete"],
            ["inspect", "probe", "diagnose-lock", "restore-readonly"],
        ),
        (
            vec!["debug", "--help"],
            ["inspect", "probe", "diagnose-lock", "restore-readonly"],
            ["create", "read", "update", "delete"],
        ),
    ] {
        let output = run(&args);
        assert!(output.status.success());
        assert_eq!(output.stderr, Vec::<u8>::new());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("Usage: shadow-realm"));
        assert!(help.contains("DISPOSABLE WINDOWS VM ONLY"));
        assert!(help.contains("Restoration is best-effort"));
        assert!(help.contains("access-denied locks: at most 8 attempts"));
        assert!(help.contains("4.5s total backoff"));
        assert!(!help.contains("--remount-snapshot"));
        for command in visible {
            assert!(has_command(&help, command), "{help}");
        }
        for command in hidden {
            assert!(!has_command(&help, command), "{help}");
        }
    }
}

#[test]
fn every_leaf_has_generated_usage_and_command_specific_flags() {
    for (command, debug, required, forbidden) in [
        (
            "create",
            false,
            vec!["--source", "--confirm-lab", "--dry-run"],
            vec!["--output", "--confirm-delete", "--remount-snapshot"],
        ),
        (
            "read",
            false,
            vec!["--file", "--output"],
            vec![
                "--source",
                "--confirm-lab",
                "--dry-run",
                "--remount-snapshot",
            ],
        ),
        (
            "update",
            false,
            vec!["--source", "--confirm-lab", "--dry-run"],
            vec!["--output", "--confirm-delete", "--remount-snapshot"],
        ),
        (
            "delete",
            false,
            vec!["--file", "--confirm-lab", "--confirm-delete", "--dry-run"],
            vec!["--source", "--output", "--remount-snapshot"],
        ),
        (
            "inspect",
            true,
            vec![],
            vec!["--file", "--confirm-lab", "--remount-snapshot"],
        ),
        (
            "diagnose-lock",
            true,
            vec!["--file", "--confirm-lab"],
            vec!["--source", "--output", "--dry-run", "--remount-snapshot"],
        ),
        (
            "probe",
            true,
            vec!["--confirm-lab"],
            vec!["--file", "--dry-run", "--remount-snapshot"],
        ),
        (
            "restore-readonly",
            true,
            vec!["--confirm-lab"],
            vec!["--file", "--dry-run", "--remount-snapshot"],
        ),
    ] {
        let mut args = vec![command, "--help"];
        if debug {
            args.insert(0, "debug");
        }
        let output = run(&args);
        assert!(output.status.success(), "{args:?}");
        assert_eq!(output.stderr, Vec::<u8>::new());
        let help = String::from_utf8(output.stdout).unwrap();
        let prefix = if debug { "debug " } else { "" };
        assert!(help.contains(&format!("Usage: shadow-realm {prefix}{command}")));
        assert!(help.contains("--snapshot <NUMBER>"));
        assert!(help.contains("--help"));
        for flag in required {
            assert!(help.contains(flag), "{flag}: {help}");
        }
        for flag in forbidden {
            assert!(!help.contains(flag), "{flag}: {help}");
        }
    }
}

#[test]
fn clap_handles_help_forms_version_and_missing_commands_without_dispatch() {
    assert_eq!(run(&["-h"]).stdout, run(&["--help"]).stdout);
    assert_eq!(
        run(&["help", "update"]).stdout,
        run(&["update", "--help"]).stdout
    );
    assert_eq!(
        run(&["help", "debug", "probe"]).stdout,
        run(&["debug", "probe", "--help"]).stdout
    );
    let version = run(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        concat!("shadow-realm ", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(version.stderr, Vec::<u8>::new());
    for args in [vec![], vec!["debug"]] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(output.stdout, Vec::<u8>::new());
        assert!(String::from_utf8(output.stderr).unwrap().contains("Usage:"));
    }
}

#[test]
fn argument_errors_use_clap_diagnostics_not_runtime_backtraces() {
    let output = Command::new(env!("CARGO_BIN_EXE_shadow-realm"))
        .args(["debug", "inspect", "--snapshot", "4294967296"])
        .env("RUST_LIB_BACKTRACE", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, Vec::<u8>::new());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("--snapshot is outside the u32 range"),
        "{error}"
    );
    assert!(
        error.contains("number too large to fit in target type"),
        "{error}"
    );
    assert!(error.contains("--help"), "{error}");
    assert!(!error.contains("Stack backtrace:"), "{error}");
}

#[test]
fn invalid_commands_flags_and_consent_fail_before_dispatch() {
    for arguments in [
        vec!["unknown-command"],
        vec!["--debug", "inspect", "--snapshot", "3"],
        vec!["debug", "inspect", "--snapshot", "3", "--debug"],
        vec!["inspect", "--snapshot", "3"],
        vec!["debug", "read", "--snapshot", "3", "--file", "target"],
        vec!["debug", "probe", "--snapshot", "3"],
        vec![
            "create",
            "--snapshot",
            "3",
            "--file",
            "target",
            "--source",
            "missing",
        ],
        vec![
            "update",
            "--snapshot",
            "3",
            "--file",
            "target",
            "--source",
            "missing",
            "--dry-run",
            "--confirm-lab",
        ],
        vec![
            "delete",
            "--snapshot",
            "3",
            "--file",
            "target",
            "--confirm-lab",
        ],
        vec!["debug", "inspect", "--snapshot", "0"],
        vec!["read", "--snapshot", "3", "--file", "..\\outside"],
        vec!["debug", "probe", "--restore-attributes-only"],
        vec![
            "debug",
            "probe",
            "--snapshot",
            "3",
            "--confirm-lab",
            "--remount-snapshot",
        ],
        vec![
            "debug",
            "restore-readonly",
            "--snapshot",
            "3",
            "--confirm-lab",
            "--remount-snapshot",
        ],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert_eq!(output.stdout, Vec::<u8>::new(), "{arguments:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("error:"), "{error}");
        assert!(error.contains("--help"), "{error}");
    }
}
