//! Buffered Win32 volume controls and reopen-based filesystem mounting.

use super::handles::{
    FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, open_file_handle, win32_error,
};
use crate::error::{Error, Result};
use crate::paths::snapshot_path;
use crate::snapshot::Volume;
use std::ffi::c_void;
use std::fs;
use std::io;
use std::num::NonZeroU32;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::path::Path;

// Windows SDK ntddvol.h: CTL_CODE('V', 13/14, METHOD_BUFFERED, FILE_ANY_ACCESS).
const SET_GPT_ATTRIBUTES: u32 = 0x0056_0034;
const GET_GPT_ATTRIBUTES: u32 = 0x0056_0038;
const DISK_IS_WRITABLE: u32 = 0x0007_0024;
const LOCK_VOLUME: u32 = 0x0009_0018;
const UNLOCK_VOLUME: u32 = 0x0009_001c;
const DISMOUNT_VOLUME: u32 = 0x0009_0020;
const ERROR_WRITE_PROTECT: i32 = 19;

// All controls below are synchronous and buffered; no pointer survives the call.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn DeviceIoControl(
        device: *mut c_void,
        code: u32,
        input: *mut c_void,
        input_size: u32,
        output: *mut c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
}

#[derive(Debug)]
pub(crate) struct SnapshotVolume {
    path: String,
    handle: Option<OwnedHandle>,
}

fn encode_attributes(attributes: u64) -> [u8; 16] {
    // VOLUME_SET_GPT_ATTRIBUTES_INFORMATION is 16 bytes. Both booleans and
    // reserved fields stay zero; the inspected driver rejects RevertOnClose.
    let mut input = [0; 16];
    input[..8].copy_from_slice(&attributes.to_le_bytes());
    input
}

impl SnapshotVolume {
    pub(crate) fn open(snapshot: NonZeroU32) -> Result<Self> {
        let path = snapshot_path(snapshot);
        let handle = Self::open_handle(&path)?;
        Ok(Self {
            path,
            handle: Some(handle),
        })
    }

    fn open_handle(path: &str) -> Result<OwnedHandle> {
        // These buffered controls use FILE_ANY_ACCESS; the driver separately
        // checks authorization. No raw-write access or privilege bypass is used.
        open_file_handle(
            Path::new(path),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            0,
        )
    }

    fn control(&self, code: u32, input: &mut [u8], output: &mut [u8]) -> io::Result<u32> {
        let handle = self.handle.as_ref().ok_or_else(|| {
            io::Error::other("Snapshot handle is unavailable after a failed reopen")
        })?;
        let input_size = u32::try_from(input.len()).map_err(io::Error::other)?;
        let output_size = u32::try_from(output.len()).map_err(io::Error::other)?;
        let input_ptr = if input.is_empty() {
            std::ptr::null_mut()
        } else {
            input.as_mut_ptr().cast()
        };
        let output_ptr = if output.is_empty() {
            std::ptr::null_mut()
        } else {
            output.as_mut_ptr().cast()
        };
        let mut returned = 0;
        // SAFETY: the owned handle stays live. Input/output slices are distinct,
        // valid for their checked lengths, and borrowed until this synchronous
        // call returns. Empty buffers and the OVERLAPPED argument are null.
        let success = unsafe {
            DeviceIoControl(
                handle.as_raw_handle(),
                code,
                input_ptr,
                input_size,
                output_ptr,
                output_size,
                &raw mut returned,
                std::ptr::null_mut(),
            )
        };
        if success == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(returned)
        }
    }
}

impl Volume for SnapshotVolume {
    fn attributes(&mut self) -> Result<u64> {
        let mut output = [0; 8];
        let returned = self
            .control(GET_GPT_ATTRIBUTES, &mut [], &mut output)
            .map_err(|error| win32_error("IOCTL_VOLUME_GET_GPT_ATTRIBUTES", error))?;
        if returned != 8 {
            return Err(format!("Attribute query returned {returned} bytes; expected 8").into());
        }
        Ok(u64::from_le_bytes(output))
    }

    fn set_attributes(&mut self, attributes: u64) -> Result<()> {
        self.control(
            SET_GPT_ATTRIBUTES,
            &mut encode_attributes(attributes),
            &mut [],
        )
        .map_err(|error| win32_error("IOCTL_VOLUME_SET_GPT_ATTRIBUTES", error))?;
        Ok(())
    }

    fn is_writable(&mut self) -> Result<bool> {
        match self.control(DISK_IS_WRITABLE, &mut [], &mut []) {
            Ok(_) => Ok(true),
            Err(error) if error.raw_os_error() == Some(ERROR_WRITE_PROTECT) => Ok(false),
            Err(error) => Err(win32_error("IOCTL_DISK_IS_WRITABLE", error)),
        }
    }

    fn lock(&mut self) -> Result<()> {
        if self.handle.is_none() {
            println!("Retrying snapshot open for restoration after an earlier reopen failure.");
            self.reopen()?;
        }
        self.control(LOCK_VOLUME, &mut [], &mut [])
            .map_err(|error| {
                win32_error(
                    "FSCTL_LOCK_VOLUME (no force-dismount will be attempted)",
                    error,
                )
            })?;
        Ok(())
    }

    fn dismount(&mut self) -> Result<()> {
        self.control(DISMOUNT_VOLUME, &mut [], &mut [])
            .map_err(|error| win32_error("FSCTL_DISMOUNT_VOLUME", error))?;
        Ok(())
    }

    fn unlock(&mut self) -> Result<()> {
        self.control(UNLOCK_VOLUME, &mut [], &mut [])
            .map_err(|error| win32_error("FSCTL_UNLOCK_VOLUME", error))?;
        Ok(())
    }

    fn reopen(&mut self) -> Result<()> {
        // Closing the locking handle also releases its lock if explicit unlock
        // failed. Do not open the replacement before closing the old handle.
        drop(self.handle.take());
        self.handle = Some(Self::open_handle(&self.path)?);
        println!("Shadow-device handle closed and reopened.");
        // A directory access requests a filesystem mount, rather than assuming
        // that opening a direct volume handle alone mounted the filesystem.
        let root = format!("{}\\", self.path);
        let directory = fs::read_dir(&root).map_err(|error| {
            Error::context("Access snapshot root to request filesystem mount", error)
        })?;
        drop(directory);
        println!("Snapshot root is accessible after reopen.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_layout_matches_sdk_and_does_not_request_revert_on_close() {
        let input = encode_attributes(0xe000_0000_0000_0000);
        assert_eq!(input.len(), 16);
        assert_eq!(&input[..8], &0xe000_0000_0000_0000u64.to_le_bytes());
        assert_eq!(&input[8..], &[0; 8]);
        assert_eq!(SET_GPT_ATTRIBUTES, (u32::from(b'V') << 16) | (13 << 2));
        assert_eq!(GET_GPT_ATTRIBUTES, (u32::from(b'V') << 16) | (14 << 2));
        assert_eq!(LOCK_VOLUME, (9 << 16) | (6 << 2));
        assert_eq!(UNLOCK_VOLUME, (9 << 16) | (7 << 2));
        assert_eq!(DISMOUNT_VOLUME, (9 << 16) | (8 << 2));
    }
}
