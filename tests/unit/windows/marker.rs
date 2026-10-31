use super::*;

#[test]
fn marker_round_trip_and_collision_leave_existing_file_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("marker.tmp");
    marker_at(path.clone()).unwrap();
    assert!(!path.exists());
    fs::write(&path, b"existing file").unwrap();
    assert!(marker_at(path.clone()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"existing file");
    directory.close().unwrap();
}
