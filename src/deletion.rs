use super::{validate_relative_file, windows::open_file_handle, Result};
use std::ffi::c_void;
use std::fs;
use std::io;
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};

const READ_ATTRIBUTES: u32 = 0x80;
const READ_DATA_OR_LIST_DIRECTORY: u32 = 1;
const DELETE_ACCESS: u32 = 0x0001_0000;
const OPEN_REPARSE_BACKUP: u32 = 0x0220_0000;
const DIRECTORY: u32 = 0x10;
const REPARSE_POINT: u32 = 0x400;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
#[derive(Default)]
struct FileInformation {
    attributes: u32,
    created: FileTime,
    accessed: FileTime,
    written: FileTime,
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct FileId {
    volume_serial: u64,
    id: [u8; 16],
}

#[derive(Debug, PartialEq, Eq)]
pub struct Identity {
    id: FileId,
    created: FileTime,
    written: FileTime,
    size: u64,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(handle: *mut c_void, information: *mut FileInformation) -> i32;
    fn GetFileInformationByHandleEx(
        handle: *mut c_void,
        class: i32,
        information: *mut c_void,
        size: u32,
    ) -> i32;
    fn SetFileInformationByHandle(
        handle: *mut c_void,
        class: i32,
        information: *mut c_void,
        size: u32,
    ) -> i32;
}

fn information(handle: &OwnedHandle) -> Result<FileInformation> {
    let mut info = FileInformation::default();
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
        return Err(format!(
            "GetFileInformationByHandle: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(info)
}

fn identity(handle: &OwnedHandle, info: &FileInformation) -> Result<Identity> {
    let mut id = FileId::default();
    if unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            18,
            (&mut id as *mut FileId).cast(),
            size_of::<FileId>() as u32,
        )
    } == 0
    {
        return Err(format!(
            "Query 128-bit file identity: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(Identity {
        id,
        created: info.created,
        written: info.written,
        size: (u64::from(info.size_high) << 32) | u64::from(info.size_low),
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
        READ_ATTRIBUTES | READ_DATA_OR_LIST_DIRECTORY,
        1,
        OPEN_REPARSE_BACKUP,
    )?;
    let info = information(&handle)?;
    if info.attributes & REPARSE_POINT != 0 {
        return Err(format!(
            "Refusing a reparse-point directory: {}",
            path.display()
        ));
    }
    if info.attributes & DIRECTORY == 0 {
        return Err(format!("Not a directory: {}", path.display()));
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
    let access =
        READ_ATTRIBUTES | READ_DATA_OR_LIST_DIRECTORY | if deleting { DELETE_ACCESS } else { 0 };
    // Exclusive access avoids deleting an actively opened file or leaving a
    // delete-pending name held by another user handle.
    let file = open_file_handle(&path, access, 0, OPEN_REPARSE_BACKUP)?;
    let info = information(&file)?;
    if info.attributes & REPARSE_POINT != 0 {
        return Err(format!("Refusing a reparse-point file: {}", path.display()));
    }
    if info.attributes & DIRECTORY != 0 {
        return Err("Only one regular file can be deleted; directories are not accepted".into());
    }
    let identity = identity(&file, &info)?;
    Ok(CheckedFile {
        file,
        parents,
        path,
        identity,
        links: info.links,
    })
}

pub fn preview(root: &str, relative: &str) -> Result<Identity> {
    let checked = open_checked(root, relative, false)?;
    println!("Selected file: {}", checked.path.display());
    println!(
        "Size: {} bytes; hard links: {}",
        checked.identity.size, checked.links
    );
    println!("File ID: {:02x?}", checked.identity.id.id);
    // All preview handles close before the volume-lock/remount operation.
    Ok(checked.identity)
}

pub fn delete_file(root: &str, relative: &str, expected: &Identity) -> Result<()> {
    let checked = open_checked(root, relative, true)?;
    if &checked.identity != expected {
        return Err(
            "Target identity, size, or timestamps changed after preview; nothing deleted".into(),
        );
    }
    let CheckedFile {
        file,
        parents,
        path,
        ..
    } = checked;
    // FILE_DISPOSITION_INFO contains one BOOLEAN (one byte), not a Win32 BOOL.
    let mut delete: u8 = 1;
    if unsafe {
        SetFileInformationByHandle(file.as_raw_handle(), 4, (&mut delete as *mut u8).cast(), 1)
    } == 0
    {
        return Err(format!(
            "Delete selected file by handle: {}",
            io::Error::last_os_error()
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
        Ok(_) => Err(
            "Deletion was requested but the selected name is still present; state is uncertain"
                .into(),
        ),
        Err(error) => Err(format!(
            "Deletion requested, but absence could not be verified: {error}"
        )),
    };
    drop(parents);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn area() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("vss-delete-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn windows_metadata_layout_matches_sdk() {
        assert_eq!(size_of::<FileInformation>(), 52);
        assert_eq!(size_of::<FileId>(), 24);
    }

    #[test]
    fn preview_is_read_only_and_deletion_removes_only_the_selected_file() {
        let root = area();
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
        fs::remove_file(sentinel).unwrap();
        fs::remove_dir(root.join("nested")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn changed_target_and_directories_are_not_deleted() {
        let root = area();
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
        fs::remove_file(target).unwrap();
        fs::remove_dir(root.join("folder")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn busy_and_readonly_files_are_not_forced_and_attributes_are_preserved() {
        let root = area();
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
        fs::remove_file(target).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn pinned_parent_cannot_be_renamed_during_the_operation() {
        let root = area();
        fs::create_dir(root.join("parent")).unwrap();
        fs::write(root.join("parent").join("file"), b"keep").unwrap();
        let checked = open_checked(root.to_str().unwrap(), r"parent\file", false).unwrap();
        assert!(fs::rename(root.join("parent"), root.join("moved")).is_err());
        assert!(
            open_file_handle(&root.join("parent"), 0x4000_0000, 7, OPEN_REPARSE_BACKUP).is_err()
        );
        drop(checked);
        drop(open_file_handle(&root.join("parent"), 0x4000_0000, 7, OPEN_REPARSE_BACKUP).unwrap());
        fs::remove_file(root.join("parent").join("file")).unwrap();
        fs::remove_dir(root.join("parent")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn junction_cannot_redirect_deletion_outside_the_selected_root() {
        let root = area();
        let outside = area();
        let sentinel = outside.join("sentinel");
        fs::write(&sentinel, b"outside").unwrap();
        let junction = root.join("junction");
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(preview(root.to_str().unwrap(), r"junction\sentinel")
            .unwrap_err()
            .contains("reparse"));
        assert!(preview(root.to_str().unwrap(), "junction")
            .unwrap_err()
            .contains("reparse"));
        let outside_id = preview(outside.to_str().unwrap(), "sentinel").unwrap();
        assert!(delete_file(root.to_str().unwrap(), r"junction\sentinel", &outside_id).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside");
        fs::remove_dir(junction).unwrap();
        fs::remove_file(sentinel).unwrap();
        fs::remove_dir(outside).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn replacement_with_same_contents_is_not_deleted() {
        let root = area();
        let target = root.join("target");
        fs::write(&target, b"same content").unwrap();
        let id = preview(root.to_str().unwrap(), "target").unwrap();
        fs::rename(&target, root.join("original")).unwrap();
        fs::write(&target, b"same content").unwrap();
        assert!(delete_file(root.to_str().unwrap(), "target", &id).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"same content");
        fs::remove_file(target).unwrap();
        fs::remove_file(root.join("original")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn deleting_one_hard_link_does_not_erase_the_other_name() {
        let root = area();
        let target = root.join("target");
        let alias = root.join("alias");
        fs::write(&target, b"shared data").unwrap();
        fs::hard_link(&target, &alias).unwrap();
        let id = preview(root.to_str().unwrap(), "target").unwrap();
        delete_file(root.to_str().unwrap(), "target", &id).unwrap();
        assert!(!target.exists());
        assert_eq!(fs::read(&alias).unwrap(), b"shared data");
        fs::remove_file(alias).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
