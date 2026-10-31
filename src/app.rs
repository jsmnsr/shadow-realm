//! Dispatches validated CLI commands to snapshot policy and Windows file operations.

#![forbid(unsafe_code)]

use crate::cli::Options;
#[cfg(windows)]
use crate::{
    cli::Command,
    paths::snapshot_path,
    snapshot::{
        Volume, check_snapshot_attributes, diagnose_lock, restore_readonly, with_writable_snapshot,
    },
    windows::{self, files},
};
#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;
#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use windows_sys::Win32::System::Ioctl::{
    GPT_BASIC_DATA_ATTRIBUTE_READ_ONLY as READ_ONLY,
    GPT_BASIC_DATA_ATTRIBUTE_SHADOW_COPY as SHADOW_COPY,
};

#[cfg(windows)]
fn run_file_command<V: Volume>(volume: &mut V, path: &str, options: &Options) -> Result<()> {
    check_snapshot_attributes(volume.attributes()?)?;
    let relative = options
        .file
        .as_deref()
        .context("File commands require --file")?;
    if options.command == Command::Read {
        return files::read_file(path, relative, options.output.as_deref().map(Path::new));
    }
    let source = options
        .source
        .as_deref()
        .map(|source| -> Result<Vec<u8>> {
            let bytes = files::load_source(Path::new(source))?;
            println!("Source: {source} ({} bytes)", bytes.len());
            Ok(bytes)
        })
        .transpose()?;
    let action: Box<dyn FnOnce() -> Result<()> + '_> = match options.command {
        Command::Create => {
            let expected = files::preview_create(path, relative)?;
            let bytes = source.context("create requires --source")?;
            Box::new(move || files::create_file(path, relative, &expected, &bytes))
        }
        Command::Update => {
            let expected = files::preview_update(path, relative)?;
            let bytes = source.context("update requires --source")?;
            Box::new(move || files::update_file(path, relative, &expected, &bytes))
        }
        Command::Delete => {
            let expected = files::preview(path, relative)?;
            Box::new(move || files::delete_file(path, relative, &expected))
        }
        _ => anyhow::bail!("Not a file command"),
    };
    if options.dry_run {
        println!("DRY RUN: no attributes or file contents changed.");
        return Ok(());
    }
    with_writable_snapshot(volume, options.remount, action)?;
    println!(
        "File operation completed; original read-only driver attributes restored. This is not rollback or proof of filesystem integrity."
    );
    Ok(())
}

#[cfg(windows)]
pub(crate) fn run(options: &Options) -> Result<()> {
    let path = snapshot_path(options.snapshot);
    if options.command == Command::Read {
        eprintln!("Target: {path}");
    } else {
        println!("Target: {path}");
    }
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
                .context("diagnose-lock requires --file")?;
            diagnose_lock(&mut volume, || files::preview(&path, relative).map(|_| ()))
        }
        Command::Create | Command::Read | Command::Update | Command::Delete => {
            run_file_command(&mut volume, &path, options)
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn run(_options: &Options) -> Result<()> {
    anyhow::bail!("This experiment requires Windows. No operation was performed.")
}

#[cfg(all(test, windows))]
#[path = "../tests/unit/app.rs"]
mod tests;
