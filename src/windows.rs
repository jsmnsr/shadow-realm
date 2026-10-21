use super::{combine, Result, Volume};
use std::ffi::c_void;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// Windows SDK ntddvol.h: CTL_CODE('V', 13/14, METHOD_BUFFERED, FILE_ANY_ACCESS).
const SET_GPT_ATTRIBUTES: u32 = 0x0056_0034;
const GET_GPT_ATTRIBUTES: u32 = 0x0056_0038;
const DISK_IS_WRITABLE: u32 = 0x0007_0024;
const LOCK_VOLUME: u32 = 0x0009_0018;
const UNLOCK_VOLUME: u32 = 0x0009_001c;
const DISMOUNT_VOLUME: u32 = 0x0009_0020;
const ERROR_WRITE_PROTECT: i32 = 19;

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

pub struct SnapshotVolume {
    path: String,
    handle: Option<OwnedHandle>,
}

fn win32_error(operation: &str, error: io::Error) -> String {
    format!("{operation}: {error} (Win32 {:?})", error.raw_os_error())
}

fn encode_attributes(attributes: u64) -> [u8; 16] {
    // VOLUME_SET_GPT_ATTRIBUTES_INFORMATION is 16 bytes. Both booleans and
    // reserved fields stay zero; the inspected driver rejects RevertOnClose.
    let mut input = [0; 16];
    input[..8].copy_from_slice(&attributes.to_le_bytes());
    input
}

pub(crate) fn open_file_handle(
    path: &Path,
    access: u32,
    share: u32,
    flags: u32,
) -> Result<OwnedHandle> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            share,
            std::ptr::null(),
            3,
            flags,
            std::ptr::null_mut(),
        )
    };
    if handle == -1isize as *mut c_void {
        return Err(win32_error(
            &format!("Open {}", path.display()),
            io::Error::last_os_error(),
        ));
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

impl SnapshotVolume {
    pub fn open(path: &str) -> Result<Self> {
        Ok(Self {
            path: path.to_string(),
            handle: Some(Self::open_handle(path)?),
        })
    }

    fn open_handle(path: &str) -> Result<OwnedHandle> {
        // These buffered controls use FILE_ANY_ACCESS; the driver separately
        // checks authorization. No raw-write access or privilege bypass is used.
        open_file_handle(Path::new(path), 0x8000_0000, 3, 0)
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
        // Buffers remain valid for this synchronous, non-overlapped request.
        let success = unsafe {
            DeviceIoControl(
                handle.as_raw_handle(),
                code,
                input_ptr,
                input_size,
                output_ptr,
                output_size,
                &mut returned,
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
        if returned != output.len() as u32 {
            return Err(format!(
                "Attribute query returned {returned} bytes; expected 8"
            ));
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
            format!("Access snapshot root to request filesystem mount: {error}")
        })?;
        drop(directory);
        println!("Snapshot root is accessible after reopen.");
        Ok(())
    }
}

struct Marker {
    path: PathBuf,
    owned: bool,
}

impl Marker {
    fn cleanup(&mut self) -> Result<()> {
        // Report the exact path if deletion fails; never delete an unowned file.
        self.owned = false;
        fs::remove_file(&self.path)
            .map_err(|error| format!("Test file may remain at {}: {error}", self.path.display()))
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
        .map_err(|error| format!("Create test file {}: {error}", path.display()))?;
    let mut marker = Marker { path, owned: true };
    // The file is captured by the closure, so it closes before marker cleanup,
    // including during panic unwinding.
    let operation = (move || {
        let mut file = file;
        let payload = b"vss-write-demo: owned disposable-VM test file\r\n";
        file.write_all(payload)
            .map_err(|error| format!("Write test file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Flush test file: {error}"))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| format!("Seek test file: {error}"))?;
        let mut observed = Vec::new();
        file.take(payload.len() as u64 + 1)
            .read_to_end(&mut observed)
            .map_err(|error| format!("Read back test file: {error}"))?;
        if observed != payload {
            return Err("Test-file contents did not match".into());
        }
        Ok(())
    })();
    combine(operation, marker.cleanup(), "TEST-FILE CLEANUP FAILED")
}

pub fn marker_probe(root: &str) -> Result<()> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock: {error}"))?
        .as_nanos();
    let name = format!("vss-write-demo-{}-{nonce}.tmp", std::process::id());
    let path = Path::new(root).join(name);
    println!("Owned test file: {}", path.display());
    marker_at(path)
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

    #[test]
    fn marker_round_trip_and_collision_leave_existing_file_untouched() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("vss-demo-unit-test-{}-{nonce}", std::process::id()));
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
