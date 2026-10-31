use super::*;
use anyhow::anyhow;
use std::io;

#[test]
fn io_context_retains_the_original_error() {
    let error = Error::new(io::Error::from_raw_os_error(5)).context("Open selected snapshot");
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().raw_os_error(),
        Some(5)
    );
    assert!(format!("{error:#}").starts_with("Open selected snapshot: "));
}

#[test]
fn operation_and_cleanup_failures_remain_observable() {
    let error = combine(
        Err(anyhow!("operation failed").context("operation context")),
        Err(anyhow!("cleanup failed").context("cleanup context")),
        "RESTORATION FAILED",
    )
    .unwrap_err();
    let display = format!("{error:#}");
    assert!(display.contains("operation context: operation failed"));
    assert!(display.contains("RESTORATION FAILED: cleanup context: cleanup failed"));
    let cause = error.downcast_ref::<CleanupError>().unwrap();
    assert_eq!(cause.operation.root_cause().to_string(), "operation failed");
    assert_eq!(cause.cleanup.root_cause().to_string(), "cleanup failed");
}

#[test]
fn successful_cleanup_does_not_hide_an_operation_failure() {
    assert!(combine(Ok(()), Ok(()), "cleanup").is_ok());
    assert_eq!(
        combine(Err(anyhow!("operation")), Ok(()), "cleanup")
            .unwrap_err()
            .to_string(),
        "operation"
    );
    let error = combine(Ok(()), Err(anyhow!("failure")), "cleanup").unwrap_err();
    assert_eq!(format!("{error:#}"), "cleanup: failure");
}
