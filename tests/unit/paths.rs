use super::*;

#[test]
fn constructs_only_exact_shadow_paths() {
    assert_eq!(
        snapshot_path(NonZeroU32::new(42).unwrap()),
        r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy42"
    );
}

#[test]
fn rejects_unsafe_relative_file_paths() {
    for value in [
        "",
        r"\Windows\a",
        r"C:\a",
        r"..\a",
        r"a\..\b",
        r"a\.\b",
        r"\\server\a",
        r"a\\b",
        "a\\",
        "a/b",
        "a:stream",
        "*.bat",
        "a?.bat",
        "a\0b",
        "a\nb",
        "a.",
        "a ",
        r"a.\b",
        "NUL",
        "con.txt",
        r"a\COM1.log",
        "LPT9",
        "CONIN$",
        "CONOUT$",
        "con .txt",
        "COM\u{b9}.txt",
    ] {
        assert!(validate_relative_file(value).is_err(), "{value:?}");
    }
    for value in [
        r"Windows\Update\rec\data.bat",
        r"vss003-20230123T020915.9536740\Windows\Update\rec\data.bat",
        ".hidden",
        "file name.txt",
    ] {
        assert!(validate_relative_file(value).is_ok(), "{value}");
    }
}

#[test]
fn relative_path_limit_counts_utf16_units_not_utf8_bytes() {
    assert!(validate_relative_file(&"a".repeat(30_000)).is_ok());
    assert!(validate_relative_file(&"a".repeat(30_001)).is_err());
    assert!(validate_relative_file(&"\u{1f600}".repeat(15_000)).is_ok());
    assert!(validate_relative_file(&format!("{}a", "\u{1f600}".repeat(15_000))).is_err());
}
