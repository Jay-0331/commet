//! Typed representation of `git status --porcelain=v1 -z` output.
//!
//! This module ships the minimal parsing the rest of E3 needs to
//! land:
//!
//! - A [`FileEntry`] carrying the path and a coarse [`FileStatus`]
//!   enum.
//! - A [`parse_porcelain`] helper covering the common single-path
//!   status codes.
//!
//! Edge cases — rename pairs (`R  old -> new`), copy pairs, full
//! conflict matrices, paths with embedded NULs — are deliberately
//! deferred to #22. The current parser will surface a Renamed entry
//! for `R` codes but only records the new path; downstream callers
//! that need both old and new paths should wait for #22.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

use super::wrappers::run_bytes;

/// Coarse status indicator for one tracked or untracked file.
///
/// `git status --porcelain=v1` reports a two-character code per
/// entry; we collapse it to a single high-level variant since the
/// caller (TUI file picker) only needs to color rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// `??` — present on disk, not in the index.
    Untracked,
    /// `A ` or ` A` — newly tracked.
    Added,
    /// `M `, ` M`, `MM`, etc. — content differs.
    Modified,
    /// `D ` or ` D` — removed.
    Deleted,
    /// `R…` — renamed (and possibly modified). #22 expands this
    /// variant with the old path.
    Renamed,
    /// Conflict markers (`AA`, `DD`, `UU`, or anything with `U`).
    Conflicted,
    /// Any code we don't recognize yet. Should be rare; tests that
    /// exercise an unfamiliar code can extend the matrix.
    Other,
}

/// One row from `git status --porcelain=v1 -z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub status: FileStatus,
}

/// Run `git status --porcelain=v1 -z` in `cwd` and parse the output.
pub fn status_porcelain(cwd: &Path) -> Result<Vec<FileEntry>> {
    let bytes = run_bytes(
        cwd,
        &[
            OsStr::new("status"),
            OsStr::new("--porcelain=v1"),
            OsStr::new("-z"),
        ],
    )?;
    parse_porcelain(&bytes)
}

/// Parse the raw bytes produced by `git status --porcelain=v1 -z`.
///
/// Entries are separated by NUL bytes. Each entry begins with a
/// two-byte status code, a single space, and then the path. Rename
/// entries are followed by an additional NUL-terminated old path —
/// for now we consume and discard it; #22 will surface both paths.
pub fn parse_porcelain(bytes: &[u8]) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    let mut chunks = bytes.split(|b| *b == 0).peekable();

    while let Some(chunk) = chunks.next() {
        if chunk.is_empty() {
            continue;
        }
        if chunk.len() < 3 {
            return Err(Error::Git(format!(
                "porcelain entry too short ({} bytes): {:?}",
                chunk.len(),
                bytes_preview(chunk),
            )));
        }
        let code = &chunk[..2];
        // Skip the separator space after the two-char code.
        let path_bytes = &chunk[3..];

        let status = classify(code);

        // Rename entries (`R…`) are followed by the old path as a
        // second NUL-terminated field. Consume and discard it for
        // now — #22 will keep it.
        if matches!(status, FileStatus::Renamed) {
            chunks.next();
        }

        let path = path_from_bytes(path_bytes)?;
        entries.push(FileEntry { path, status });
    }

    Ok(entries)
}

fn classify(code: &[u8]) -> FileStatus {
    match code {
        b"??" => FileStatus::Untracked,
        b"A " | b" A" | b"AM" => FileStatus::Added,
        b"D " | b" D" => FileStatus::Deleted,
        b"M " | b" M" | b"MM" | b"AM " => FileStatus::Modified,
        // Any rename / copy combination.
        c if c[0] == b'R' || c[0] == b'C' => FileStatus::Renamed,
        // Conflict matrix: anything containing 'U', or AA, DD.
        c if c.contains(&b'U') || c == b"AA" || c == b"DD" => FileStatus::Conflicted,
        _ => {
            // Common fallthrough: any non-space, non-'?' marker in
            // either column → modified.
            let stage = code[0] != b' ' && code[0] != b'?';
            let work = code[1] != b' ' && code[1] != b'?';
            if stage || work {
                FileStatus::Modified
            } else {
                FileStatus::Other
            }
        }
    }
}

fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf> {
    // On Unix paths can be any byte sequence; on Windows they're
    // typically UTF-16 but git emits UTF-8 here. We accept any
    // valid UTF-8 string and surface a clear error otherwise so
    // callers can decide whether to skip or fail.
    let s = std::str::from_utf8(bytes).map_err(|_| {
        Error::Git(format!(
            "porcelain path is not utf-8: {:?}",
            bytes_preview(bytes),
        ))
    })?;
    Ok(PathBuf::from(s))
}

fn bytes_preview(bytes: &[u8]) -> String {
    let head: Vec<u8> = bytes.iter().take(32).copied().collect();
    String::from_utf8_lossy(&head).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_entries() {
        assert!(parse_porcelain(b"").unwrap().is_empty());
    }

    #[test]
    fn single_untracked_entry() {
        // Format: "?? notes.md\0"
        let bytes = b"?? notes.md\0";
        let entries = parse_porcelain(bytes).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("notes.md"));
        assert_eq!(entries[0].status, FileStatus::Untracked);
    }

    #[test]
    fn single_modified_entry() {
        let bytes = b" M src/main.rs\0";
        let entries = parse_porcelain(bytes).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("src/main.rs"));
        assert_eq!(entries[0].status, FileStatus::Modified);
    }

    #[test]
    fn single_added_entry() {
        let bytes = b"A  src/new.rs\0";
        let entries = parse_porcelain(bytes).unwrap();
        assert_eq!(entries[0].status, FileStatus::Added);
    }

    #[test]
    fn single_deleted_entry() {
        let bytes = b" D src/gone.rs\0";
        let entries = parse_porcelain(bytes).unwrap();
        assert_eq!(entries[0].status, FileStatus::Deleted);
    }

    #[test]
    fn conflict_codes_classified_as_conflicted() {
        for code in [&b"UU"[..], b"AA", b"DD", b"AU", b"UD"] {
            let mut bytes = code.to_vec();
            bytes.extend_from_slice(b" conflict.txt\0");
            let entries = parse_porcelain(&bytes).unwrap();
            assert_eq!(
                entries[0].status,
                FileStatus::Conflicted,
                "code {:?} should be Conflicted",
                String::from_utf8_lossy(code),
            );
        }
    }

    #[test]
    fn rename_entry_consumes_old_path_and_records_new() {
        // Format: "R  new.rs\0old.rs\0"
        let bytes = b"R  new.rs\0old.rs\0";
        let entries = parse_porcelain(bytes).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, FileStatus::Renamed);
        // For now we keep the *new* path; #22 will expose the pair.
        assert_eq!(entries[0].path, PathBuf::from("new.rs"));
    }

    #[test]
    fn multiple_mixed_entries() {
        let bytes = b" M src/a.rs\0?? notes.md\0A  src/b.rs\0 D removed.rs\0";
        let entries = parse_porcelain(bytes).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(
            entries.iter().map(|e| e.status).collect::<Vec<_>>(),
            vec![
                FileStatus::Modified,
                FileStatus::Untracked,
                FileStatus::Added,
                FileStatus::Deleted,
            ],
        );
    }

    #[test]
    fn too_short_chunk_errors_clearly() {
        // A chunk shorter than the "XY path" minimum is a parser bug
        // upstream or a malformed payload; surface it explicitly.
        let bytes = b"??\0";
        let err = parse_porcelain(bytes).unwrap_err();
        assert!(matches!(err, Error::Git(msg) if msg.contains("too short")));
    }
}
