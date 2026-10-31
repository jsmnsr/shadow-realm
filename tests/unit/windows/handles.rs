use super::*;
use windows_sys::Win32::Foundation::GENERIC_READ;

#[test]
fn embedded_nul_is_rejected_before_windows_can_truncate_the_path() {
    let error = open_file_handle(Path::new("unused\0suffix"), GENERIC_READ, 0, 0).unwrap_err();
    assert!(format!("{error:#}").contains("embedded NUL"));
}
