//! Synchronous Win32 file opening with uniquely owned, automatically closed handles.

use anyhow::{Error, Result};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ACCESS_RIGHTS, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, OPEN_EXISTING,
};

pub(super) fn win32_error(operation: &str, error: io::Error) -> Error {
    let context = format!("{operation} (Win32 {:?})", error.raw_os_error());
    Error::new(error).context(context)
}

pub(super) fn open_file_handle(
    path: &Path,
    access: FILE_ACCESS_RIGHTS,
    share: FILE_SHARE_MODE,
    flags: FILE_FLAGS_AND_ATTRIBUTES,
) -> Result<OwnedHandle> {
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if name.contains(&0) {
        anyhow::bail!("Win32 file paths cannot contain embedded NUL characters");
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
    use windows_sys::Win32::Foundation::GENERIC_READ;

    #[test]
    fn embedded_nul_is_rejected_before_windows_can_truncate_the_path() {
        let error = open_file_handle(Path::new("unused\0suffix"), GENERIC_READ, 0, 0).unwrap_err();
        assert!(format!("{error:#}").contains("embedded NUL"));
    }
}
