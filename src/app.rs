//! Dispatches validated CLI commands to snapshot policy and Windows file operations.

#![forbid(unsafe_code)]

use crate::{
    cli::{Command, Options},
    paths::snapshot_path,
    snapshot::{
        Volume, check_snapshot_attributes, diagnose_lock, restore_readonly, with_writable_snapshot,
    },
    windows::{self, files},
};
use anyhow::{Context, Result};
use std::path::Path;
use windows_sys::Win32::System::Ioctl::{
    GPT_BASIC_DATA_ATTRIBUTE_READ_ONLY as READ_ONLY,
    GPT_BASIC_DATA_ATTRIBUTE_SHADOW_COPY as SHADOW_COPY,
};

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
    match options.command {
        Command::Create => {
            let expected = files::preview_create(path, relative)?;
            let bytes = source.context("create requires --source")?;
            mutate_file(volume, options.dry_run, || {
                files::create_file(path, relative, &expected, &bytes)
            })
        }
        Command::Update => {
            let expected = files::preview_update(path, relative)?;
            let bytes = source.context("update requires --source")?;
            mutate_file(volume, options.dry_run, || {
                files::update_file(path, relative, &expected, &bytes)
            })
        }
        Command::Delete => {
            let expected = files::preview(path, relative)?;
            mutate_file(volume, options.dry_run, || {
                files::delete_file(path, relative, &expected)
            })
        }
        _ => anyhow::bail!("Not a file command"),
    }
}

fn mutate_file<V: Volume>(
    volume: &mut V,
    dry_run: bool,
    action: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if dry_run {
        println!("DRY RUN: no attributes or file contents changed.");
        return Ok(());
    }
    with_writable_snapshot(volume, action)?;
    println!(
        "File operation completed; original read-only driver attributes restored. This is not rollback or proof of filesystem integrity."
    );
    Ok(())
}

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
            with_writable_snapshot(&mut volume, || windows::marker::marker_probe(&path))?;
            println!(
                "Probe succeeded: test file verified and removed; original attributes restored."
            );
            Ok(())
        }
        Command::RestoreReadonly => restore_readonly(&mut volume),
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

#[cfg(test)]
#[path = "../tests/unit/app.rs"]
mod tests;
