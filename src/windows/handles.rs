//! Synchronous Win32 file opening with uniquely owned, automatically closed handles.

use crate::error::{Error, Result};
use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;

// WinBase.h: open only existing objects, never create or truncate them.
const OPEN_EXISTING: u32 = 3;
const INVALID_HANDLE_VALUE: *mut c_void = -1_isize as *mut c_void;
pub(super) const GENERIC_READ: u32 = 0x8000_0000;
pub(super) const FILE_SHARE_READ: u32 = 1;
pub(super) const FILE_SHARE_WRITE: u32 = 2;

// Synchronous calls only: names must be NUL-terminated and remain alive until return.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const c_void,
        disposition: u32,
        flags: u32,
        template: *mut c_void,
    ) -> *mut c_void;
}

pub(super) fn win32_error(operation: &str, error: io::Error) -> Error {
    Error::context(
        format!("{operation} (Win32 {:?})", error.raw_os_error()),
        error,
    )
}

pub(super) fn open_file_handle(
    path: &Path,
    access: u32,
    share: u32,
    flags: u32,
) -> Result<OwnedHandle> {
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if name.contains(&0) {
        return Err("Win32 file paths cannot contain embedded NUL characters".into());
    }
    name.push(0);
    // SAFETY: name is a live, NUL-terminated UTF-16 buffer. Optional security
    // and template pointers are null; OPEN_EXISTING never creates an object.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            share,
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(win32_error(
            &format!("Open {}", path.display()),
            io::Error::last_os_error(),
        ));
    }
    // SAFETY: CreateFileW returned a valid handle, uniquely owned here. Ownership
    // transfers exactly once to OwnedHandle, which closes it with CloseHandle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_nul_is_rejected_before_windows_can_truncate_the_path() {
        let error = open_file_handle(Path::new("unused\0suffix"), GENERIC_READ, 0, 0).unwrap_err();
        assert!(error.to_string().contains("embedded NUL"));
    }
}
