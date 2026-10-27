//! Creates, verifies, and removes one uniquely owned probe file without overwriting existing files.

#![forbid(unsafe_code)]

use crate::error::{Error, Result, combine};
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
        fs::remove_file(&self.path).map_err(|error| {
            Error::context(
                format!("Test file may remain at {}", self.path.display()),
                error,
            )
        })
    }
}

impl Drop for Marker {
    fn drop(&mut self) {
        if self.owned {
            if let Err(error) = self.cleanup() {
                eprintln!("TEST-FILE CLEANUP FAILED: {error}");
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
        .map_err(|error| Error::context(format!("Create test file {}", path.display()), error))?;
    let mut marker = Marker { path, owned: true };
    // The file is captured by the closure, so it closes before marker cleanup,
    // including during panic unwinding.
    let operation = (move || {
        let mut file = file;
        let payload = b"shadow-realm: owned disposable-VM test file\r\n";
        file.write_all(payload)
            .map_err(|error| Error::context("Write test file", error))?;
        file.sync_all()
            .map_err(|error| Error::context("Flush test file", error))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| Error::context("Seek test file", error))?;
        let mut observed = Vec::new();
        file.take(payload.len() as u64 + 1)
            .read_to_end(&mut observed)
            .map_err(|error| Error::context("Read back test file", error))?;
        if observed != payload {
            return Err("Test-file contents did not match".into());
        }
        Ok(())
    })();
    combine(operation, marker.cleanup(), "TEST-FILE CLEANUP FAILED")
}

pub(crate) fn marker_probe(root: &str) -> Result<()> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::context("System clock", error))?
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
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "shadow-realm-unit-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("marker.tmp");
        marker_at(path.clone()).unwrap();
        assert!(!path.exists());
        fs::write(&path, b"existing file").unwrap();
        assert!(marker_at(path.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing file");
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
