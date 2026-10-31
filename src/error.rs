//! Combines operation and cleanup failures without discarding either error chain.

#![forbid(unsafe_code)]

use anyhow::{Error, Result};
use std::fmt;

// Error::source exposes one chain; retain cleanup separately for diagnostics.
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
            "Operation and cleanup failed\n{}: {:#}",
            self.label, self.cleanup
        )
    }
}

impl std::error::Error for CleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.operation.as_ref())
    }
}

pub(crate) fn combine(operation: Result<()>, cleanup: Result<()>, label: &str) -> Result<()> {
    match (operation, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error.context(label.to_owned())),
        (Err(operation), Err(cleanup)) => Err(CleanupError {
            operation,
            cleanup,
            label: label.to_owned(),
        }
        .into()),
    }
}

#[cfg(test)]
#[path = "../tests/unit/error.rs"]
mod tests;
