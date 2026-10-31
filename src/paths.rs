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
#[path = "../tests/unit/paths.rs"]
mod tests;
