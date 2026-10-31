//! Snapshot transition policy and best-effort restoration through mockable volume controls.
//!
//! Locking must succeed before dismount or attribute changes. Errors and unwinding
//! attempt cleanup, but neither restoration nor read-only flags guarantee filesystem integrity.

#![forbid(unsafe_code)]

use crate::error::combine;
use anyhow::{Context, Result};

use windows_sys::Win32::System::Ioctl::{
    GPT_BASIC_DATA_ATTRIBUTE_HIDDEN as HIDDEN,
    GPT_BASIC_DATA_ATTRIBUTE_NO_DRIVE_LETTER as NO_DRIVE_LETTER,
    GPT_BASIC_DATA_ATTRIBUTE_READ_ONLY as READ_ONLY,
    GPT_BASIC_DATA_ATTRIBUTE_SHADOW_COPY as SHADOW_COPY,
};

const KNOWN_ATTRIBUTES: u64 = NO_DRIVE_LETTER | HIDDEN | SHADOW_COPY | READ_ONLY;

pub(crate) trait Volume {
    fn attributes(&mut self) -> Result<u64>;
    fn set_attributes(&mut self, attributes: u64) -> Result<()>;
    fn is_writable(&mut self) -> Result<bool>;
    fn lock(&mut self) -> Result<()>;
    fn dismount(&mut self) -> Result<()>;
    fn unlock(&mut self) -> Result<()>;
    fn reopen(&mut self) -> Result<()>;
}

pub(crate) fn check_snapshot_attributes(attributes: u64) -> Result<()> {
    if attributes & (SHADOW_COPY | NO_DRIVE_LETTER) != (SHADOW_COPY | NO_DRIVE_LETTER) {
        anyhow::bail!(
            "Driver did not report the expected shadow-copy attributes; refusing mutation"
        );
    }
    if attributes & !KNOWN_ATTRIBUTES != 0 {
        anyhow::bail!("Unrecognized attribute bits in {attributes:#018x}; refusing mutation");
    }
    Ok(())
}

fn diagnostic_lock<V: Volume>(
    volume: &mut V,
    stage: &str,
    failures: &mut Vec<String>,
) -> Result<()> {
    println!("{stage}: attempting FSCTL_LOCK_VOLUME");
    match volume.lock() {
        Ok(()) => {
            println!("{stage}: LOCK SUCCEEDED");
            volume.unlock().with_context(|| {
                format!(
                    "{stage}: UNLOCK FAILED; stopping. The volume handle closes on command exit"
                )
            })?;
            println!("{stage}: UNLOCK SUCCEEDED");
        }
        Err(error) => {
            let message = format!("{stage}: LOCK FAILED: {error:#}");
            eprintln!("{message}");
            failures.push(message);
        }
    }
    Ok(())
}

pub(crate) fn diagnose_lock<V: Volume>(
    volume: &mut V,
    preview: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let original = volume.attributes()?;
    check_snapshot_attributes(original)?;
    if original & READ_ONLY == 0 || volume.is_writable()? {
        anyhow::bail!("Lock diagnosis requires an initially read-only snapshot");
    }
    println!(
        "Lock-only diagnosis: no attribute changes, dismounts, or deletion will be requested."
    );
    let operation = (|| {
        let mut failures = Vec::new();
        diagnostic_lock(volume, "BEFORE PREVIEW", &mut failures)?;
        println!(
            "PREVIEW: opening the selected file and its parents, then closing all preview handles."
        );
        if let Err(error) = preview() {
            failures.push(format!("PREVIEW FAILED: {error:#}"));
            anyhow::bail!(failures.join("\n"));
        }
        // The same volume handle is used on both sides of the preview.
        if let Err(error) = diagnostic_lock(volume, "AFTER PREVIEW", &mut failures) {
            failures.push(format!("{error:#}"));
            anyhow::bail!(failures.join("\n"));
        }
        if failures.is_empty() {
            println!(
                "Both lock/unlock checks succeeded. This does not test write enablement or deletion."
            );
            Ok(())
        } else {
            Err(anyhow::anyhow!(failures.join("\n")))
        }
    })();
    let verification = (|| {
        let observed = volume.attributes()?;
        if observed != original || volume.is_writable()? {
            anyhow::bail!(
                "Snapshot state changed during diagnosis: attributes {observed:#018x}; no automatic restoration was attempted"
            );
        }
        println!("Driver attributes unchanged: {observed:#018x}; driver remains read-only.");
        Ok(())
    })();
    combine(operation, verification, "STATE VERIFICATION FAILED")
}

struct RemountGuard<'a, V: Volume> {
    volume: &'a mut V,
    armed: bool,
}

impl<V: Volume> RemountGuard<'_, V> {
    fn finish(&mut self) -> Result<()> {
        self.armed = false;
        let unlock = self.volume.unlock();
        let reopen = self.volume.reopen();
        combine(unlock, reopen, "SNAPSHOT REOPEN FAILED")
    }
}

impl<V: Volume> Drop for RemountGuard<'_, V> {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self.finish() {
                eprintln!("REMOUNT CLEANUP FAILED during unwinding: {error:?}");
            }
        }
    }
}

fn change_attributes<V: Volume>(volume: &mut V, attributes: u64, remount: bool) -> Result<()> {
    if !remount {
        anyhow::bail!("Attribute-only transitions are disabled; remount consent is required");
    }
    println!("Locking the selected shadow device; busy snapshots will not be force-dismounted.");
    volume.lock()?;
    let mut guard = RemountGuard {
        volume,
        armed: true,
    };
    let operation = (|| {
        println!("Dismounting the locked shadow filesystem.");
        guard.volume.dismount()?;
        guard.volume.set_attributes(attributes)
    })();
    // Always release the successful lock and close/reopen the old handle,
    // even if dismount, the attribute change, or explicit unlocking fails.
    combine(operation, guard.finish(), "SNAPSHOT UNLOCK/REOPEN FAILED")
}

struct RestoreGuard<'a, V: Volume> {
    volume: &'a mut V,
    original: u64,
    armed: bool,
    remount: bool,
}

impl<V: Volume> RestoreGuard<'_, V> {
    fn restore(&mut self) -> Result<()> {
        self.armed = false;
        change_attributes(self.volume, self.original, self.remount)?;
        let observed = self.volume.attributes()?;
        if observed != self.original {
            anyhow::bail!(
                "Expected restored attributes {:#018x}, observed {observed:#018x}",
                self.original
            );
        }
        if self.volume.is_writable()? {
            anyhow::bail!("Driver still reports writable after restoring read-only attributes");
        }
        println!(
            "Remount completed; original driver attributes restored: {observed:#018x}. Filesystem integrity is not established."
        );
        Ok(())
    }
}

impl<V: Volume> Drop for RestoreGuard<'_, V> {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self.restore() {
                eprintln!("RESTORATION FAILED during unwinding: {error:?}");
                eprintln!(
                    "Use debug restore-readonly on this snapshot or revert the disposable VM checkpoint."
                );
            }
        }
    }
}

pub(crate) fn with_writable_snapshot<V: Volume>(
    volume: &mut V,
    remount: bool,
    action: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if !remount {
        anyhow::bail!("Writable operations require remount-based transitions and restoration");
    }
    let original = volume.attributes()?;
    check_snapshot_attributes(original)?;
    if original & READ_ONLY == 0 {
        anyhow::bail!(
            "Snapshot is already writable; refusing operation. Use debug restore-readonly if appropriate."
        );
    }
    if volume.is_writable()? {
        anyhow::bail!("Writable status contradicts READ_ONLY; refusing mutation");
    }
    println!("Original attributes: {original:#018x}");
    let desired = original & !READ_ONLY;
    // Arm before the request: even a failed setter may have partially changed state.
    let mut guard = RestoreGuard {
        volume,
        original,
        armed: true,
        remount,
    };
    let operation = (|| {
        println!("Requesting temporary writable attributes: {desired:#018x}");
        change_attributes(guard.volume, desired, remount).context("Write-enable request failed")?;
        let observed = guard.volume.attributes()?;
        if observed != desired {
            anyhow::bail!(
                "Attribute transition not observed: expected {desired:#018x}, got {observed:#018x}"
            );
        }
        if !guard.volume.is_writable()? {
            anyhow::bail!("Driver still reports write protection after the attribute change");
        }
        println!("Driver reports writable; attempting the requested file operation.");
        action()
    })();
    let restoration = guard.restore();
    combine(
        operation,
        restoration,
        "RESTORATION FAILED; snapshot state is uncertain. Use debug restore-readonly or revert the VM checkpoint",
    )
}

pub(crate) fn restore_readonly<V: Volume>(volume: &mut V, remount: bool) -> Result<()> {
    let before = volume.attributes()?;
    check_snapshot_attributes(before)?;
    let desired = before | READ_ONLY;
    change_attributes(volume, desired, remount).context("Read-only recovery failed; snapshot state is uncertain. Do not force dismount or use attribute-only recovery; revert the disposable VM checkpoint")?;
    let observed = volume.attributes()?;
    if observed != desired || volume.is_writable()? {
        anyhow::bail!("Read-only restoration not confirmed; observed attributes {observed:#018x}");
    }
    println!(
        "Remount completed; driver reports read-only; attributes: {observed:#018x}. Filesystem integrity is not established."
    );
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/snapshot.rs"]
mod tests;
