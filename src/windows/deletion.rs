//! Pins non-reparse paths and deletes one identity-checked file through its exclusive handle.

use super::handles::{open_file_handle, win32_error};
use crate::paths::validate_relative_file;
use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_READ,
    FileDispositionInfo, FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    SetFileInformationByHandle,
};

const OPEN_REPARSE_BACKUP: u32 = FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    volume_serial: u64,
    id: [u8; 16],
    created: u64,
    written: u64,
    size: u64,
}

fn filetime_ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn information(handle: &OwnedHandle) -> Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: handle remains live and info is writable, properly aligned storage
    // matching BY_HANDLE_FILE_INFORMATION for the duration of the call.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &raw mut info) } == 0 {
        return Err(win32_error(
            "GetFileInformationByHandle",
            io::Error::last_os_error(),
        ));
    }
    Ok(info)
}

fn identity(handle: &OwnedHandle, info: &BY_HANDLE_FILE_INFORMATION) -> Result<Identity> {
    let mut id = FILE_ID_INFO::default();
    let size = u32::try_from(size_of_val(&id)).context("File identity buffer size")?;
    // SAFETY: FileIdInfo selects the SDK FILE_ID_INFO layout; id is aligned,
    // writable, and lives through the synchronous call. The handle is borrowed.
    if unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileIdInfo,
            std::ptr::from_mut(&mut id).cast(),
            size,
        )
    } == 0
    {
        return Err(win32_error(
            "Query 128-bit file identity",
            io::Error::last_os_error(),
        ));
    }
    Ok(Identity {
        volume_serial: id.VolumeSerialNumber,
        id: id.FileId.Identifier,
        created: filetime_ticks(info.ftCreationTime),
        written: filetime_ticks(info.ftLastWriteTime),
        size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
    })
}

struct CheckedFile {
    file: OwnedHandle,
    parents: Vec<OwnedHandle>,
    path: PathBuf,
    identity: Identity,
    links: u32,
}

fn pin_directory(path: &Path) -> Result<OwnedHandle> {
    // Deny write/delete sharing while checking and using the path. The held
    // handles prevent ancestor replacement or reparse changes during deletion.
    // Attribute-only handles do not participate in read/write sharing checks.
    let handle = open_file_handle(
        path,
        FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY,
        FILE_SHARE_READ,
        OPEN_REPARSE_BACKUP,
    )?;
    let info = information(&handle)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        anyhow::bail!("Refusing a reparse-point directory: {}", path.display());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        anyhow::bail!("Not a directory: {}", path.display());
    }
    Ok(handle)
}

fn open_checked(root: &str, relative: &str, deleting: bool) -> Result<CheckedFile> {
    validate_relative_file(relative)?;
    let mut path = PathBuf::from(format!("{root}\\"));
    let mut parents = vec![pin_directory(&path)?];
    let mut components = relative.split('\\').peekable();
    while let Some(component) = components.next() {
        path.push(component);
        if components.peek().is_some() {
            parents.push(pin_directory(&path)?);
        }
    }
    let access = FILE_READ_ATTRIBUTES | FILE_READ_DATA | if deleting { DELETE } else { 0 };
    // Exclusive access avoids deleting an actively opened file or leaving a
    // delete-pending name held by another user handle.
    let file = open_file_handle(&path, access, 0, OPEN_REPARSE_BACKUP)?;
    let info = information(&file)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        anyhow::bail!("Refusing a reparse-point file: {}", path.display());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        anyhow::bail!("Only one regular file can be deleted; directories are not accepted");
    }
    let identity = identity(&file, &info)?;
    Ok(CheckedFile {
        file,
        parents,
        path,
        identity,
        links: info.nNumberOfLinks,
    })
}

pub(crate) fn preview(root: &str, relative: &str) -> Result<Identity> {
    let checked = open_checked(root, relative, false)?;
    println!("Selected file: {}", checked.path.display());
    println!(
        "Size: {} bytes; hard links: {}",
        checked.identity.size, checked.links
    );
    println!("File ID: {:02x?}", checked.identity.id);
    // All preview handles close before the volume-lock/remount operation.
    Ok(checked.identity)
}

pub(crate) fn delete_file(root: &str, relative: &str, expected: &Identity) -> Result<()> {
    let checked = open_checked(root, relative, true)?;
    if &checked.identity != expected {
        anyhow::bail!(
            "Target identity, size, or timestamps changed after preview; nothing deleted"
        );
    }
    let CheckedFile {
        file,
        parents,
        path,
        ..
    } = checked;
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let size = u32::try_from(size_of_val(&disposition)).context("File disposition buffer size")?;
    // SAFETY: file is a live handle opened with DELETE access. The selected class
    // reads the SDK disposition structure, which stays live until the call returns.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            std::ptr::from_ref(&disposition).cast(),
            size,
        )
    } == 0
    {
        return Err(win32_error(
            "Delete selected file by handle",
            io::Error::last_os_error(),
        ));
    }
    drop(file);
    let result = match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            println!(
                "Deletion completed and selected name is absent: {}",
                path.display()
            );
            Ok(())
        }
        Ok(_) => Err(anyhow::anyhow!(
            "Deletion was requested but the selected name is still present; state is uncertain"
        )),
        Err(error) => Err(anyhow::Error::from(error)
            .context("Deletion requested, but absence could not be verified")),
    };
    drop(parents);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Foundation::GENERIC_WRITE;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_WRITE};

    #[test]
    fn native_timestamps_preserve_both_halves() {
        let time = FILETIME {
            dwLowDateTime: 0x0123_4567,
            dwHighDateTime: 0x89ab_cdef,
        };
        assert_eq!(filetime_ticks(time), 0x89ab_cdef_0123_4567);
    }

    #[test]
    fn preview_is_read_only_and_deletion_removes_only_the_selected_file() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path();
        fs::create_dir(root.join("nested")).unwrap();
        let target = root.join("nested").join("data.bat");
        fs::write(&target, b"test-only content").unwrap();
        let sentinel = root.join("keep.txt");
        fs::write(&sentinel, b"unchanged").unwrap();
        let name = root.to_str().unwrap();
        let id = preview(name, r"nested\data.bat").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"test-only content");
        delete_file(name, r"nested\data.bat", &id).unwrap();
        assert!(!target.exists());
        assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
        area.close().unwrap();
    }

    #[test]
    fn changed_target_and_directories_are_not_deleted() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path();
        let target = root.join("target");
        fs::write(&target, b"before").unwrap();
        let name = root.to_str().unwrap();
        let id = preview(name, "target").unwrap();
        fs::write(&target, b"changed content").unwrap();
        assert!(delete_file(name, "target", &id).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"changed content");
        fs::create_dir(root.join("folder")).unwrap();
        assert!(preview(name, "folder").is_err());
        assert!(preview(name, "missing").is_err());
        area.close().unwrap();
    }

    #[test]
    fn busy_and_readonly_files_are_not_forced_and_attributes_are_preserved() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path();
        let target = root.join("target");
        fs::write(&target, b"keep").unwrap();
        let name = root.to_str().unwrap();
        let id = preview(name, "target").unwrap();
        let busy = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&target)
            .unwrap();
        assert!(preview(name, "target").is_err());
        assert!(delete_file(name, "target", &id).is_err());
        drop(busy);
        let original_permissions = fs::metadata(&target).unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&target, permissions).unwrap();
        assert!(delete_file(name, "target", &id).is_err());
        assert!(fs::metadata(&target).unwrap().permissions().readonly());
        fs::set_permissions(&target, original_permissions).unwrap();
        area.close().unwrap();
    }

    #[test]
    fn pinned_parent_cannot_be_renamed_during_the_operation() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path();
        fs::create_dir(root.join("parent")).unwrap();
        fs::write(root.join("parent").join("file"), b"keep").unwrap();
        let checked = open_checked(root.to_str().unwrap(), r"parent\file", false).unwrap();
        assert!(fs::rename(root.join("parent"), root.join("moved")).is_err());
        assert!(
            open_file_handle(
                &root.join("parent"),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                OPEN_REPARSE_BACKUP
            )
            .is_err()
        );
        drop(checked);
        drop(
            open_file_handle(
                &root.join("parent"),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                OPEN_REPARSE_BACKUP,
            )
            .unwrap(),
        );
        area.close().unwrap();
    }

    #[test]
    fn junction_cannot_redirect_deletion_outside_the_selected_root() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path();
        let outside = tempfile::tempdir().unwrap();
        let outside_root = outside.path();
        let sentinel = outside_root.join("sentinel");
        fs::write(&sentinel, b"outside").unwrap();
        let junction = root.join("junction");
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside_root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            preview(root.to_str().unwrap(), r"junction\sentinel")
                .unwrap_err()
                .chain()
                .any(|cause| cause.to_string().contains("reparse"))
        );
        assert!(
            preview(root.to_str().unwrap(), "junction")
                .unwrap_err()
                .chain()
                .any(|cause| cause.to_string().contains("reparse"))
        );
        let outside_id = preview(outside_root.to_str().unwrap(), "sentinel").unwrap();
        assert!(delete_file(root.to_str().unwrap(), r"junction\sentinel", &outside_id).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside");
        fs::remove_dir(junction).unwrap();
        area.close().unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside");
        outside.close().unwrap();
    }

    #[test]
    fn replacement_with_same_contents_is_not_deleted() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path();
        let target = root.join("target");
        fs::write(&target, b"same content").unwrap();
        let id = preview(root.to_str().unwrap(), "target").unwrap();
        fs::rename(&target, root.join("original")).unwrap();
        fs::write(&target, b"same content").unwrap();
        assert!(delete_file(root.to_str().unwrap(), "target", &id).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"same content");
        area.close().unwrap();
    }

    #[test]
    fn deleting_one_hard_link_does_not_erase_the_other_name() {
        let area = tempfile::tempdir().unwrap();
        let root = area.path();
        let target = root.join("target");
        let alias = root.join("alias");
        fs::write(&target, b"shared data").unwrap();
        fs::hard_link(&target, &alias).unwrap();
        let id = preview(root.to_str().unwrap(), "target").unwrap();
        delete_file(root.to_str().unwrap(), "target", &id).unwrap();
        assert!(!target.exists());
        assert_eq!(fs::read(&alias).unwrap(), b"shared data");
        area.close().unwrap();
    }
}
