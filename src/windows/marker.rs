//! Creates, verifies, and removes one uniquely owned probe file without overwriting existing files.

#![forbid(unsafe_code)]

use crate::error::combine;
use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
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
    // The file is captured by the closure, so it closes before marker cleanup,
    // including during panic unwinding.
    let operation = (move || {
        let mut file = file;
        let payload = b"shadow-realm: owned disposable-VM test file\r\n";
        file.write_all(payload).context("Write test file")?;
        file.sync_all().context("Flush test file")?;
        file.seek(SeekFrom::Start(0)).context("Seek test file")?;
        let mut observed = Vec::new();
        file.take(payload.len() as u64 + 1)
            .read_to_end(&mut observed)
            .context("Read back test file")?;
        if observed != payload {
            anyhow::bail!("Test-file contents did not match");
        }
        Ok(())
    })();
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
mod tests {
    use super::*;

    #[test]
    fn marker_round_trip_and_collision_leave_existing_file_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("marker.tmp");
        marker_at(path.clone()).unwrap();
        assert!(!path.exists());
        fs::write(&path, b"existing file").unwrap();
        assert!(marker_at(path.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing file");
        directory.close().unwrap();
    }
}
