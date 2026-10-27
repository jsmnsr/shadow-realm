//! Exact shadow-device paths and rejection of ambiguous relative file names.

#![forbid(unsafe_code)]

use anyhow::Result;
use std::num::NonZeroU32;

// Reserve space for the device prefix below Windows' extended-path length limit.
const MAX_RELATIVE_UTF16: usize = 30_000;

pub(crate) fn snapshot_path(number: NonZeroU32) -> String {
    format!(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy{number}")
}

pub(crate) fn validate_relative_file(path: &str) -> Result<()> {
    if path.is_empty() || path.encode_utf16().count() > MAX_RELATIVE_UTF16 {
        anyhow::bail!(
            "--file must be a nonempty snapshot-relative path shorter than 30001 UTF-16 units"
        );
    }
    if path
        .chars()
        .any(|ch| ch.is_control() || "/:*?\"<>|".contains(ch))
    {
        anyhow::bail!(
            "--file cannot contain a drive, stream, wildcard, control character, or forward slash"
        );
    }
    for component in path.split('\\') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
        {
            anyhow::bail!(
                "--file must not be rooted, contain traversal/empty components, or end a component with a dot/space"
            );
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.chars().count() == 1
                    && suffix
                        .chars()
                        .next()
                        .is_some_and(|ch| matches!(ch, '1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'))
            })
        }) {
            anyhow::bail!("--file cannot contain reserved DOS device names");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
