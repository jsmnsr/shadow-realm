//! Buffered Win32 volume controls and reopen-based filesystem mounting.

use super::handles::{open_file_handle, win32_error};
use crate::error::{Error, Result};
use crate::paths::snapshot_path;
use crate::snapshot::Volume;
use std::fs;
use std::io;
use std::num::NonZeroU32;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::path::Path;
use windows_sys::Win32::Foundation::{ERROR_WRITE_PROTECT, GENERIC_READ};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_READ, FILE_SHARE_WRITE, IOCTL_VOLUME_GET_GPT_ATTRIBUTES,
    IOCTL_VOLUME_SET_GPT_ATTRIBUTES, VOLUME_SET_GPT_ATTRIBUTES_INFORMATION,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, FSCTL_UNLOCK_VOLUME, IOCTL_DISK_IS_WRITABLE,
    VOLUME_GET_GPT_ATTRIBUTES_INFORMATION,
};

#[derive(Debug)]
pub(crate) struct SnapshotVolume {
    path: String,
    handle: Option<OwnedHandle>,
}

fn attribute_request(attributes: u64) -> VOLUME_SET_GPT_ATTRIBUTES_INFORMATION {
    // Affect only the selected device; RevertOnClose is not supported by the
    // inspected driver. Restoration is performed explicitly by snapshot policy.
    VOLUME_SET_GPT_ATTRIBUTES_INFORMATION {
        GptAttributes: attributes,
        RevertOnClose: false,
        ApplyToAllConnectedVolumes: false,
        Reserved1: 0,
        Reserved2: 0,
    }
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

    fn control(
        &self,
        code: u32,
        input: Option<&VOLUME_SET_GPT_ATTRIBUTES_INFORMATION>,
        output: Option<&mut VOLUME_GET_GPT_ATTRIBUTES_INFORMATION>,
    ) -> io::Result<u32> {
        let handle = self.handle.as_ref().ok_or_else(|| {
            io::Error::other("Snapshot handle is unavailable after a failed reopen")
        })?;
        let (input_ptr, input_size) = match input {
            Some(value) => (
                std::ptr::from_ref(value).cast(),
                u32::try_from(size_of_val(value)).map_err(io::Error::other)?,
            ),
            None => (std::ptr::null(), 0),
        };
        let (output_ptr, output_size) = match output {
            Some(value) => (
                std::ptr::from_mut(value).cast(),
                u32::try_from(size_of_val(value)).map_err(io::Error::other)?,
            ),
            None => (std::ptr::null_mut(), 0),
        };
        let mut returned = 0;
        // SAFETY: the owned handle and SDK structures stay live through this
        // synchronous call. The private callers select only the controls matching
        // these layouts; the output contains only a u64, with no invalid bit patterns.
        // Absent buffers and the OVERLAPPED argument are null.
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
        let mut output = VOLUME_GET_GPT_ATTRIBUTES_INFORMATION::default();
        let returned = self
            .control(IOCTL_VOLUME_GET_GPT_ATTRIBUTES, None, Some(&mut output))
            .map_err(|error| win32_error("IOCTL_VOLUME_GET_GPT_ATTRIBUTES", error))?;
        let expected = size_of_val(&output);
        if usize::try_from(returned).ok() != Some(expected) {
            return Err(
                format!("Attribute query returned {returned} bytes; expected {expected}").into(),
            );
        }
        Ok(output.GptAttributes)
    }

    fn set_attributes(&mut self, attributes: u64) -> Result<()> {
        self.control(
            IOCTL_VOLUME_SET_GPT_ATTRIBUTES,
            Some(&attribute_request(attributes)),
            None,
        )
        .map_err(|error| win32_error("IOCTL_VOLUME_SET_GPT_ATTRIBUTES", error))?;
        Ok(())
    }

    fn is_writable(&mut self) -> Result<bool> {
        match self.control(IOCTL_DISK_IS_WRITABLE, None, None) {
            Ok(_) => Ok(true),
            Err(error)
                if error
                    .raw_os_error()
                    .and_then(|code| u32::try_from(code).ok())
                    == Some(ERROR_WRITE_PROTECT) =>
            {
                Ok(false)
            }
            Err(error) => Err(win32_error("IOCTL_DISK_IS_WRITABLE", error)),
        }
    }

    fn lock(&mut self) -> Result<()> {
        if self.handle.is_none() {
            println!("Retrying snapshot open for restoration after an earlier reopen failure.");
            self.reopen()?;
        }
        self.control(FSCTL_LOCK_VOLUME, None, None)
            .map_err(|error| {
                win32_error(
                    "FSCTL_LOCK_VOLUME (no force-dismount will be attempted)",
                    error,
                )
            })?;
        Ok(())
    }

    fn dismount(&mut self) -> Result<()> {
        self.control(FSCTL_DISMOUNT_VOLUME, None, None)
            .map_err(|error| win32_error("FSCTL_DISMOUNT_VOLUME", error))?;
        Ok(())
    }

    fn unlock(&mut self) -> Result<()> {
        self.control(FSCTL_UNLOCK_VOLUME, None, None)
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
    fn attribute_request_is_scoped_to_one_volume_without_revert_on_close() {
        let input = attribute_request(0xe000_0000_0000_0000);
        assert_eq!(input.GptAttributes, 0xe000_0000_0000_0000);
        assert!(!input.RevertOnClose);
        assert!(!input.ApplyToAllConnectedVolumes);
        assert_eq!(input.Reserved1, 0);
        assert_eq!(input.Reserved2, 0);
    }
}
