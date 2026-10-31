//! Creates, verifies, and removes one uniquely owned probe file without overwriting existing files.

#![forbid(unsafe_code)]

use super::files::write_contents;
use crate::error::combine;
use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct Marker {
    path: PathBuf,
    owned: bool,
}

impl Marker {
    fn cleanup(&mut self) -> Result<()> {
        // Report the exact path if deletion fails; never delete an unowned file.
        self.owned = false;
        fs::remove_file(&self.path)
            .with_context(|| format!("Test file may remain at {}", self.path.display()))
    }
}

impl Drop for Marker {
    fn drop(&mut self) {
        if self.owned {
            if let Err(error) = self.cleanup() {
                eprintln!("TEST-FILE CLEANUP FAILED: {error:?}");
            }
        }
    }
}

fn marker_at(path: PathBuf) -> Result<()> {
    // create_new prevents collisions from overwriting or deleting existing files.
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("Create test file {}", path.display()))?;
    let mut marker = Marker { path, owned: true };
    // Move the file into an inner scope so it closes before marker cleanup,
    // including during panic unwinding.
    let operation = {
        let mut file = file;
        write_contents(
            &mut file,
            b"shadow-realm: owned disposable-VM test file\r\n",
        )
        .context("Write and verify test file")
    };
    combine(operation, marker.cleanup(), "TEST-FILE CLEANUP FAILED")
}

pub(crate) fn marker_probe(root: &str) -> Result<()> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock")?
        .as_nanos();
    let name = format!("shadow-realm-{}-{nonce}.tmp", std::process::id());
    let path = Path::new(root).join(name);
    println!("Owned test file: {}", path.display());
    marker_at(path)
}

#[cfg(test)]
#[path = "../../tests/unit/windows/marker.rs"]
mod tests;
