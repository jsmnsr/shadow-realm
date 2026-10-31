use super::{validate_relative_file, windows::open_file_handle, Result};
use std::ffi::c_void;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::mem::size_of;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};

const READ_ATTRIBUTES: u32 = 0x80;
const READ_DATA_OR_LIST_DIRECTORY: u32 = 1;
const DELETE_ACCESS: u32 = 0x0001_0000;
const WRITE_DATA: u32 = 2;
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

pub struct CreationTarget {
    parents: Vec<FileId>,
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
    // handles prevent ancestor replacement or reparse changes during file operations.
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

fn checked_path(root: &str, relative: &str) -> Result<(PathBuf, Vec<OwnedHandle>)> {
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
    Ok((path, parents))
}

fn open_checked(root: &str, relative: &str, extra_access: u32) -> Result<CheckedFile> {
    let (path, parents) = checked_path(root, relative)?;
    let access = READ_ATTRIBUTES | READ_DATA_OR_LIST_DIRECTORY | extra_access;
    // Exclusive access avoids deleting an actively opened file or leaving a
    // delete-pending name held by another user handle.
    let file = open_file_handle(&path, access, 0, OPEN_REPARSE_BACKUP)?;
    let info = information(&file)?;
    if info.attributes & REPARSE_POINT != 0 {
        return Err(format!("Refusing a reparse-point file: {}", path.display()));
    }
    if info.attributes & DIRECTORY != 0 {
        return Err("Only regular files are accepted; directories are not supported".into());
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
    let checked = open_checked(root, relative, 0)?;
    show_preview(&checked);
    // All preview handles close before the volume-lock/remount operation.
    Ok(checked.identity)
}

fn show_preview(checked: &CheckedFile) {
    println!("Selected file: {}", checked.path.display());
    println!(
        "Size: {} bytes; hard links: {}",
        checked.identity.size, checked.links
    );
    println!("File ID: {:02x?}", checked.identity.id.id);
}

fn require_single_link(checked: &CheckedFile) -> Result<()> {
    if checked.links != 1 {
        return Err(
            "Update requires exactly one hard link; refusing to change other file names".into(),
        );
    }
    Ok(())
}

pub fn preview_update(root: &str, relative: &str) -> Result<Identity> {
    let checked = open_checked(root, relative, 0)?;
    require_single_link(&checked)?;
    show_preview(&checked);
    Ok(checked.identity)
}

fn parent_ids(parents: &[OwnedHandle]) -> Result<Vec<FileId>> {
    parents
        .iter()
        .map(|parent| identity(parent, &information(parent)?).map(|identity| identity.id))
        .collect()
}

pub fn preview_create(root: &str, relative: &str) -> Result<CreationTarget> {
    let (path, parents) = checked_path(root, relative)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Check new file {}: {error}", path.display())),
        Ok(_) => return Err(format!("Create target already exists: {}", path.display())),
    }
    println!("New file: {} (parents must already exist)", path.display());
    Ok(CreationTarget {
        parents: parent_ids(&parents)?,
    })
}

pub fn load_source(path: &Path) -> Result<Vec<u8>> {
    // Buffer and close the source before remounting, even if it is in the shadow.
    let mut file = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|error| format!("Open source {}: {error}", path.display()))?;
    if !file
        .metadata()
        .map_err(|error| format!("Query source: {error}"))?
        .is_file()
    {
        return Err("--source must name a regular file".into());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("Read source {}: {error}", path.display()))?;
    Ok(bytes)
}

fn write_contents(file: &mut fs::File, bytes: &[u8]) -> Result<()> {
    file.write_all(bytes)
        .map_err(|error| format!("Write contents: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Flush contents: {error}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Seek for verification: {error}"))?;
    let mut buffer = [0; 64 * 1024];
    for chunk in bytes.chunks(buffer.len()) {
        let observed = &mut buffer[..chunk.len()];
        file.read_exact(observed)
            .map_err(|error| format!("Read back contents: {error}"))?;
        if observed != chunk {
            return Err("Read-back contents do not match the source".into());
        }
    }
    if file
        .read(&mut buffer[..1])
        .map_err(|error| format!("Verify end of file: {error}"))?
        != 0
    {
        return Err("Read-back file is longer than the source".into());
    }
    Ok(())
}

pub fn create_file(
    root: &str,
    relative: &str,
    expected: &CreationTarget,
    bytes: &[u8],
) -> Result<()> {
    let (path, parents) = checked_path(root, relative)?;
    if parent_ids(&parents)? != expected.parents {
        return Err("Parent directory identity changed after preview; nothing created".into());
    }
    // CREATE_NEW fails for any existing name, including reparse points.
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .share_mode(0)
        .custom_flags(OPEN_REPARSE_BACKUP)
        .open(&path)
        .map_err(|error| format!("Create new file {}: {error}", path.display()))?;
    let result = write_contents(&mut file, bytes).map_err(|error| {
        format!(
            "{error}; a new file with incomplete contents may remain at {}",
            path.display()
        )
    });
    drop(file);
    drop(parents);
    result
}

pub fn update_file(root: &str, relative: &str, expected: &Identity, bytes: &[u8]) -> Result<()> {
    let checked = open_checked(root, relative, WRITE_DATA)?;
    if &checked.identity != expected {
        return Err(
            "Target identity, size, or timestamps changed after preview; nothing updated".into(),
        );
    }
    require_single_link(&checked)?;
    let CheckedFile {
        file,
        parents,
        path,
        ..
    } = checked;
    let mut file = fs::File::from(file);
    let result = (|| {
        file.set_len(0).map_err(|error| format!("Truncate selected file: {error}"))?;
        write_contents(&mut file, bytes)
    })().map_err(|error| {
        format!("{error}; contents may be partially changed at {}. Read-only restoration is not rollback", path.display())
    });
    drop(file);
    drop(parents);
    result
}

fn copy_contents(checked: CheckedFile, output: &mut impl Write) -> Result<()> {
    let CheckedFile { file, parents, .. } = checked;
    let result = io::copy(&mut fs::File::from(file), output)
        .and_then(|_| output.flush())
        .map_err(|error| format!("Read/copy contents (output may be incomplete): {error}"));
    drop(parents);
    result
}

pub fn read_file(root: &str, relative: &str, output: Option<&Path>) -> Result<()> {
    let checked = open_checked(root, relative, 0)?;
    if let Some(path) = output {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| format!("Create new output {}: {error}", path.display()))?;
        copy_contents(checked, &mut file)?;
        file.sync_all()
            .map_err(|error| format!("Flush output {}: {error}", path.display()))
    } else {
        copy_contents(checked, &mut io::stdout().lock())
    }
}

pub fn delete_file(root: &str, relative: &str, expected: &Identity) -> Result<()> {
    let checked = open_checked(root, relative, DELETE_ACCESS)?;
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn area() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "shadow-realm-delete-test-{}-{nonce}",
            std::process::id()
        ));
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
        let checked = open_checked(root.to_str().unwrap(), r"parent\file", 0).unwrap();
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

    #[test]
    fn binary_create_read_and_update_round_trip_including_empty_and_shorter_files() {
        let root = area();
        fs::create_dir(root.join("nested")).unwrap();
        let name = root.to_str().unwrap();
        let payload: Vec<_> = (0..150_000).map(|i| (i % 256) as u8).collect();
        let expected = preview_create(name, r"nested\data.bin").unwrap();
        create_file(name, r"nested\data.bin", &expected, &payload).unwrap();
        assert_eq!(fs::read(root.join(r"nested\data.bin")).unwrap(), payload);
        for bytes in [&b"\0\xffshort"[..], &b""[..], &payload[..]] {
            let id = preview_update(name, r"nested\data.bin").unwrap();
            update_file(name, r"nested\data.bin", &id, bytes).unwrap();
            let mut observed = Vec::new();
            copy_contents(
                open_checked(name, r"nested\data.bin", 0).unwrap(),
                &mut observed,
            )
            .unwrap();
            assert_eq!(observed, bytes);
        }
        let output = root.join("export.bin");
        read_file(name, r"nested\data.bin", Some(&output)).unwrap();
        assert_eq!(fs::read(&output).unwrap(), payload);
        assert!(read_file(name, r"nested\data.bin", Some(&output)).is_err());
        assert_eq!(fs::read(&output).unwrap(), payload);
        let empty = preview_create(name, "empty").unwrap();
        create_file(name, "empty", &empty, &[]).unwrap();
        assert!(fs::read(root.join("empty")).unwrap().is_empty());
        fs::remove_file(root.join("empty")).unwrap();
        fs::remove_file(output).unwrap();
        fs::remove_file(root.join(r"nested\data.bin")).unwrap();
        fs::remove_dir(root.join("nested")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn create_never_overwrites_existing_names_and_rechecks_parent_identity() {
        let root = area();
        let name = root.to_str().unwrap();
        fs::create_dir(root.join("parent")).unwrap();
        let expected = preview_create(name, r"parent\new").unwrap();
        fs::write(root.join(r"parent\new"), b"keep").unwrap();
        assert!(preview_create(name, r"parent\new").is_err());
        assert!(preview_create(name, "parent").is_err());
        assert!(create_file(name, r"parent\new", &expected, b"overwrite").is_err());
        assert_eq!(fs::read(root.join(r"parent\new")).unwrap(), b"keep");
        fs::remove_file(root.join(r"parent\new")).unwrap();
        fs::rename(root.join("parent"), root.join("original")).unwrap();
        fs::create_dir(root.join("parent")).unwrap();
        assert!(create_file(name, r"parent\new", &expected, b"wrong parent")
            .unwrap_err()
            .contains("identity"));
        assert!(!root.join(r"parent\new").exists());
        assert!(preview_create(name, r"missing\new").is_err());
        assert!(preview_create(name, r"..\escape").is_err());
        fs::remove_dir(root.join("parent")).unwrap();
        fs::remove_dir(root.join("original")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn update_requires_unchanged_existing_single_link_file() {
        let root = area();
        let name = root.to_str().unwrap();
        let target = root.join("target");
        fs::write(&target, b"before").unwrap();
        let id = preview_update(name, "target").unwrap();
        fs::write(&target, b"changed contents").unwrap();
        assert!(update_file(name, "target", &id, b"replacement").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"changed contents");
        let id = preview_update(name, "target").unwrap();
        fs::rename(&target, root.join("original")).unwrap();
        fs::write(&target, b"changed contents").unwrap();
        assert!(update_file(name, "target", &id, b"replacement").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"changed contents");
        let id = preview_update(name, "target").unwrap();
        fs::hard_link(&target, root.join("alias")).unwrap();
        assert!(preview_update(name, "target")
            .unwrap_err()
            .contains("hard link"));
        assert!(update_file(name, "target", &id, b"replacement")
            .unwrap_err()
            .contains("hard link"));
        assert_eq!(fs::read(root.join("alias")).unwrap(), b"changed contents");
        assert!(update_file(name, "missing", &id, b"replacement").is_err());
        assert!(!root.join("missing").exists());
        assert!(preview_update(name, "missing").is_err());
        fs::create_dir(root.join("folder")).unwrap();
        assert!(preview_update(name, "folder").is_err());
        assert!(read_file(name, "folder", Some(&root.join("output"))).is_err());
        assert!(!root.join("output").exists());
        fs::remove_dir(root.join("folder")).unwrap();
        fs::remove_file(target).unwrap();
        fs::remove_file(root.join("alias")).unwrap();
        fs::remove_file(root.join("original")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn update_and_read_respect_busy_and_readonly_files() {
        let root = area();
        let name = root.to_str().unwrap();
        let target = root.join("target");
        fs::write(&target, b"keep").unwrap();
        let id = preview_update(name, "target").unwrap();
        let busy = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&target)
            .unwrap();
        assert!(update_file(name, "target", &id, b"replace").is_err());
        assert!(read_file(name, "target", Some(&root.join("output"))).is_err());
        drop(busy);
        let original = fs::metadata(&target).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&target, readonly).unwrap();
        assert!(update_file(name, "target", &id, b"replace").is_err());
        read_file(name, "target", Some(&root.join("output"))).unwrap();
        assert!(fs::metadata(&target).unwrap().permissions().readonly());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
        fs::set_permissions(&target, original).unwrap();
        fs::remove_file(root.join("output")).unwrap();
        fs::remove_file(target).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn junction_cannot_redirect_create_read_or_update() {
        let root = area();
        let outside = area();
        let name = root.to_str().unwrap();
        let junction = root.join("junction");
        fs::create_dir(&junction).unwrap();
        let new = preview_create(name, r"junction\new").unwrap();
        fs::remove_dir(&junction).unwrap();
        fs::write(outside.join("sentinel"), b"outside").unwrap();
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
        assert!(preview_create(name, r"junction\new").is_err());
        assert!(create_file(name, r"junction\new", &new, b"escape").is_err());
        assert!(!outside.join("new").exists());
        assert!(read_file(name, r"junction\sentinel", Some(&root.join("output"))).is_err());
        assert!(!root.join("output").exists());
        let id = preview_update(outside.to_str().unwrap(), "sentinel").unwrap();
        assert!(update_file(name, r"junction\sentinel", &id, b"escape").is_err());
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
        fs::remove_dir(junction).unwrap();
        fs::remove_file(outside.join("sentinel")).unwrap();
        fs::remove_dir(outside).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn source_is_regular_binary_data_and_is_closed_after_loading() {
        let root = area();
        let source = root.join("source");
        fs::write(&source, b"\0\xffsource").unwrap();
        assert_eq!(load_source(&source).unwrap(), b"\0\xffsource");
        assert!(load_source(&root).is_err());
        assert!(load_source(&root.join("missing")).is_err());
        fs::remove_file(source).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn read_reports_output_failures_and_releases_all_handles() {
        struct FailingOutput;
        impl Write for FailingOutput {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("simulated output failure"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let root = area();
        fs::write(root.join("target"), b"contents").unwrap();
        let checked = open_checked(root.to_str().unwrap(), "target", 0).unwrap();
        assert!(copy_contents(checked, &mut FailingOutput)
            .unwrap_err()
            .contains("output failure"));
        fs::remove_file(root.join("target")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
