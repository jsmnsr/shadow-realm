//! Pins non-reparse paths for identity-checked, binary-safe file operations.

use super::handles::{open_file_handle, win32_error};
use crate::paths::validate_relative_file;
use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ACCESS_RIGHTS, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
    FILE_READ_DATA, FILE_SHARE_READ, FILE_WRITE_DATA, FileDispositionInfo, FileIdInfo,
    GetFileInformationByHandle, GetFileInformationByHandleEx, SetFileInformationByHandle,
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

#[derive(Debug)]
pub(crate) struct CreationTarget {
    parents: Vec<(u64, [u8; 16])>,
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
    // handles prevent ancestor replacement or reparse changes during file operations.
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

fn open_checked(
    root: &str,
    relative: &str,
    extra_access: FILE_ACCESS_RIGHTS,
) -> Result<CheckedFile> {
    let (path, parents) = checked_path(root, relative)?;
    let access = FILE_READ_ATTRIBUTES | FILE_READ_DATA | extra_access;
    // Exclusive access avoids deleting an actively opened file or leaving a
    // delete-pending name held by another user handle.
    let file = open_file_handle(&path, access, 0, OPEN_REPARSE_BACKUP)?;
    let info = information(&file)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        anyhow::bail!("Refusing a reparse-point file: {}", path.display());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        anyhow::bail!("Only regular files are accepted; directories are not supported");
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
    println!("File ID: {:02x?}", checked.identity.id);
}

fn require_single_link(checked: &CheckedFile) -> Result<()> {
    if checked.links != 1 {
        anyhow::bail!("Update requires exactly one hard link; refusing to change other file names");
    }
    Ok(())
}

pub(crate) fn preview_update(root: &str, relative: &str) -> Result<Identity> {
    let checked = open_checked(root, relative, 0)?;
    require_single_link(&checked)?;
    show_preview(&checked);
    Ok(checked.identity)
}

fn parent_ids(parents: &[OwnedHandle]) -> Result<Vec<(u64, [u8; 16])>> {
    parents
        .iter()
        .map(|parent| {
            let identity = identity(parent, &information(parent)?)?;
            Ok((identity.volume_serial, identity.id))
        })
        .collect()
}

pub(crate) fn preview_create(root: &str, relative: &str) -> Result<CreationTarget> {
    let (path, parents) = checked_path(root, relative)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(
                anyhow::Error::from(error).context(format!("Check new file {}", path.display()))
            );
        }
        Ok(_) => anyhow::bail!("Create target already exists: {}", path.display()),
    }
    println!("New file: {} (parents must already exist)", path.display());
    Ok(CreationTarget {
        parents: parent_ids(&parents)?,
    })
}

pub(crate) fn load_source(path: &Path) -> Result<Vec<u8>> {
    // Buffer and close the source before remounting, even if it is in the shadow.
    let mut file = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .with_context(|| format!("Open source {}", path.display()))?;
    if !file.metadata().context("Query source")?.is_file() {
        anyhow::bail!("--source must name a regular file");
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("Read source {}", path.display()))?;
    Ok(bytes)
}

fn write_contents(file: &mut fs::File, bytes: &[u8]) -> Result<()> {
    file.write_all(bytes).context("Write contents")?;
    file.sync_all().context("Flush contents")?;
    file.seek(SeekFrom::Start(0))
        .context("Seek for verification")?;
    let mut buffer = [0; 8 * 1024];
    for chunk in bytes.chunks(buffer.len()) {
        let observed = &mut buffer[..chunk.len()];
        file.read_exact(observed).context("Read back contents")?;
        if observed != chunk {
            anyhow::bail!("Read-back contents do not match the source");
        }
    }
    if file.read(&mut buffer[..1]).context("Verify end of file")? != 0 {
        anyhow::bail!("Read-back file is longer than the source");
    }
    Ok(())
}

pub(crate) fn create_file(
    root: &str,
    relative: &str,
    expected: &CreationTarget,
    bytes: &[u8],
) -> Result<()> {
    let (path, parents) = checked_path(root, relative)?;
    if parent_ids(&parents)? != expected.parents {
        anyhow::bail!("Parent directory identity changed after preview; nothing created");
    }
    // CREATE_NEW fails for any existing name, including reparse points.
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .share_mode(0)
        .custom_flags(OPEN_REPARSE_BACKUP)
        .open(&path)
        .with_context(|| format!("Create new file {}", path.display()))?;
    let result = write_contents(&mut file, bytes).with_context(|| {
        format!(
            "A new file with incomplete contents may remain at {}",
            path.display()
        )
    });
    drop(file);
    drop(parents);
    result
}

pub(crate) fn update_file(
    root: &str,
    relative: &str,
    expected: &Identity,
    bytes: &[u8],
) -> Result<()> {
    let checked = open_checked(root, relative, FILE_WRITE_DATA)?;
    if &checked.identity != expected {
        anyhow::bail!(
            "Target identity, size, or timestamps changed after preview; nothing updated"
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
        file.set_len(0).context("Truncate selected file")?;
        write_contents(&mut file, bytes)
    })()
    .with_context(|| {
        format!(
            "Contents may be partially changed at {}. Read-only restoration is not rollback",
            path.display()
        )
    });
    drop(file);
    drop(parents);
    result
}

fn copy_contents(checked: CheckedFile, output: &mut impl Write) -> Result<()> {
    let CheckedFile { file, parents, .. } = checked;
    let result = io::copy(&mut fs::File::from(file), output)
        .and_then(|_| output.flush())
        .context("Read/copy contents (output may be incomplete)");
    drop(parents);
    result
}

pub(crate) fn read_file(root: &str, relative: &str, output: Option<&Path>) -> Result<()> {
    let checked = open_checked(root, relative, 0)?;
    if let Some(path) = output {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .with_context(|| format!("Create new output {}", path.display()))?;
        copy_contents(checked, &mut file)?;
        file.sync_all()
            .with_context(|| format!("Flush output {}", path.display()))
    } else {
        copy_contents(checked, &mut io::stdout().lock())
    }
}

pub(crate) fn delete_file(root: &str, relative: &str, expected: &Identity) -> Result<()> {
    let checked = open_checked(root, relative, DELETE)?;
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
#[path = "../../tests/unit/windows/files.rs"]
mod tests;
