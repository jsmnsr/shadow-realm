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
