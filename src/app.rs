//! Dispatches validated CLI commands to snapshot policy and Windows file operations.

#![forbid(unsafe_code)]

use crate::cli::Options;
use crate::error::Result;
#[cfg(windows)]
use crate::{
    cli::Command,
    paths::snapshot_path,
    snapshot::{
        READ_ONLY, SHADOW_COPY, Volume, check_snapshot_attributes, diagnose_lock, restore_readonly,
        with_writable_snapshot,
    },
    windows::{self, deletion},
};

#[cfg(windows)]
pub(crate) fn run(options: &Options) -> Result<()> {
    let path = snapshot_path(options.snapshot);
    println!("Target: {path}");
    let mut volume = windows::volume::SnapshotVolume::open(options.snapshot)?;
    match options.command {
        Command::Inspect => {
            let attributes = volume.attributes()?;
            println!("GPT attributes: {attributes:#018x}");
            println!("SHADOW_COPY: {}", attributes & SHADOW_COPY != 0);
            println!("READ_ONLY: {}", attributes & READ_ONLY != 0);
            println!("Driver writable status: {}", volume.is_writable()?);
            Ok(())
        }
        Command::Probe => {
            with_writable_snapshot(&mut volume, options.remount, || {
                windows::marker::marker_probe(&path)
            })?;
            println!(
                "Probe succeeded: test file verified and removed; original attributes restored."
            );
            Ok(())
        }
        Command::RestoreReadonly => restore_readonly(&mut volume, options.remount),
        Command::DiagnoseLock => {
            let relative = options
                .file
                .as_deref()
                .ok_or("diagnose-lock requires --file")?;
            diagnose_lock(&mut volume, || {
                deletion::preview(&path, relative).map(|_| ())
            })
        }
        Command::Delete => {
            check_snapshot_attributes(volume.attributes()?)?;
            let relative = options.file.as_deref().ok_or("delete requires --file")?;
            let expected = deletion::preview(&path, relative)?;
            if options.dry_run {
                println!("DRY RUN: no attributes changed and no file deleted.");
                return Ok(());
            }
            with_writable_snapshot(&mut volume, options.remount, || {
                deletion::delete_file(&path, relative, &expected)
            })?;
            println!(
                "Selected file name deleted; original driver attributes restored. This does not restore file contents or filesystem metadata."
            );
            Ok(())
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn run(_options: &Options) -> Result<()> {
    Err("This experiment requires Windows. No operation was performed.".into())
}
