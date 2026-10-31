//! Black-box CLI checks that never request a valid shadow-device operation.

#![forbid(unsafe_code)]

use std::process::{Command, Output};

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_shadow-realm"))
        .args(arguments)
        .output()
        .expect("built Shadow Realm executable must start")
}

#[test]
fn help_names_the_utility_and_preserves_safety_flags() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.starts_with(concat!(
        "Shadow Realm ",
        env!("CARGO_PKG_VERSION"),
        " -- DISPOSABLE WINDOWS VM ONLY"
    )));
    for expected in [
        "shadow-realm create",
        "shadow-realm read",
        "shadow-realm update",
        "shadow-realm delete",
        "--confirm-lab",
        "--confirm-delete",
    ] {
        assert!(help.contains(expected), "help must include {expected}");
    }
    for hidden in ["inspect", "diagnose-lock", "probe", "restore-readonly"] {
        assert!(!help.contains(&format!("shadow-realm {hidden}")));
    }
    assert!(!help.contains("--remount-snapshot"));
    assert_eq!(output.stderr, Vec::<u8>::new());
}

#[test]
fn debug_namespace_has_separate_help() {
    let output = run(&["debug", "--help"]);
    assert!(output.status.success());
    assert_eq!(output.stderr, Vec::<u8>::new());
    let help = String::from_utf8(output.stdout.clone()).unwrap();
    for command in ["inspect", "diagnose-lock", "probe", "restore-readonly"] {
        assert!(help.contains(&format!("shadow-realm debug {command}")));
    }
    for command in ["create", "read", "update", "delete"] {
        assert!(!help.contains(&format!("shadow-realm {command}")));
    }
    assert_eq!(run(&["debug"]).stdout, output.stdout);
    assert_eq!(run(&["debug", "-h"]).stdout, output.stdout);
}

#[test]
fn no_arguments_show_help_without_opening_a_device() {
    let output = run(&[]);
    assert!(output.status.success());
    assert_eq!(output.stdout, run(&["--help"]).stdout);
    assert_eq!(output.stdout, run(&["-h"]).stdout);
    assert_eq!(output.stderr, Vec::<u8>::new());
}

#[test]
fn error_output_preserves_causes_and_opt_in_backtraces() {
    for backtrace in ["0", "1"] {
        let output = Command::new(env!("CARGO_BIN_EXE_shadow-realm"))
            .args(["debug", "inspect", "--snapshot", "4294967296"])
            .env("RUST_LIB_BACKTRACE", backtrace)
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
        assert_eq!(
            error.contains("Stack backtrace:"),
            backtrace == "1",
            "{error}"
        );
    }
}

#[test]
fn old_debug_flags_and_misplaced_commands_fail_before_dispatch() {
    for arguments in [
        vec!["--debug", "--help"],
        vec!["--debug", "inspect", "--snapshot", "3"],
        vec!["inspect", "--snapshot", "3", "--debug"],
        vec!["inspect", "--snapshot", "3"],
        vec!["debug", "read", "--snapshot", "3", "--file", "target"],
        vec!["debug", "probe", "--snapshot", "3"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert_eq!(output.stdout, Vec::<u8>::new());
        let help = if arguments[0] == "debug" {
            "Use debug --help"
        } else {
            "Use --help"
        };
        assert!(String::from_utf8(output.stderr).unwrap().contains(help));
    }
}

#[test]
fn invalid_invocations_return_usage_errors_before_dispatch() {
    for arguments in [
        vec!["unknown-command"],
        vec!["inspect", "--snapshot", "not-a-device"],
        vec!["inspect", "--snapshot", "0"],
        vec!["delete", "--snapshot", "0", "--file", "..\\outside"],
        vec!["probe", "--restore-attributes-only"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert_eq!(output.stdout, Vec::<u8>::new(), "{arguments:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("Use --help for usage."), "{error}");
    }
}
