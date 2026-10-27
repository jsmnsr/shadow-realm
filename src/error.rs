//! Application error reporting that preserves operation and cleanup failures.

#![forbid(unsafe_code)]

use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;

pub(crate) type Result<T> = std::result::Result<T, Error>;

/// An application failure with optional upstream cause and opt-in backtrace.
#[derive(Debug)]
pub(crate) struct Error {
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    backtrace: Backtrace,
}

impl Error {
    pub(crate) fn context(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
            backtrace: Backtrace::capture(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(source) = &self.source {
            if !self.message.is_empty() {
                formatter.write_str(": ")?;
            }
            write!(formatter, "{source}")?;
        }
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(formatter, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self {
            message,
            source: None,
            backtrace: Backtrace::capture(),
        }
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

// The standard Error trait exposes one primary source. Keep both failures in
// this private cause so Display and Debug retain the cleanup error as well.
#[derive(Debug)]
struct CleanupError {
    operation: Error,
    cleanup: Error,
    label: String,
}

impl fmt::Display for CleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}\n{}: {}",
            self.operation, self.label, self.cleanup
        )
    }
}

impl std::error::Error for CleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.operation)
    }
}

pub(crate) fn combine(operation: Result<()>, cleanup: Result<()>, label: &str) -> Result<()> {
    match (operation, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(Error::context(label, error)),
        (Err(error), Err(cleanup_error)) => {
            let cause = CleanupError {
                operation: error,
                cleanup: cleanup_error,
                label: label.to_owned(),
            };
            Err(Error {
                message: String::new(),
                source: Some(Box::new(cause)),
                backtrace: Backtrace::capture(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::io;

    #[test]
    fn io_context_retains_the_original_error() {
        let error = Error::context("Open selected snapshot", io::Error::from_raw_os_error(5));
        let source = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
        assert_eq!(source.raw_os_error(), Some(5));
        assert!(error.to_string().starts_with("Open selected snapshot: "));
    }

    #[test]
    fn operation_and_cleanup_failures_remain_observable() {
        let error = combine(
            Err("operation failed".into()),
            Err("cleanup failed".into()),
            "RESTORATION FAILED",
        )
        .unwrap_err();
        let display = error.to_string();
        assert!(display.starts_with("operation failed"));
        assert!(display.contains("RESTORATION FAILED: cleanup failed"));
        let cause = error
            .source()
            .unwrap()
            .downcast_ref::<CleanupError>()
            .unwrap();
        assert!(cause.operation.to_string().starts_with("operation failed"));
        assert!(cause.cleanup.to_string().starts_with("cleanup failed"));
    }

    #[test]
    fn successful_cleanup_does_not_hide_an_operation_failure() {
        assert!(combine(Ok(()), Ok(()), "cleanup").is_ok());
        assert!(
            combine(Err("operation".into()), Ok(()), "cleanup")
                .unwrap_err()
                .to_string()
                .starts_with("operation")
        );
        assert!(
            combine(Ok(()), Err("failure".into()), "cleanup")
                .unwrap_err()
                .to_string()
                .starts_with("cleanup: failure")
        );
    }
}
