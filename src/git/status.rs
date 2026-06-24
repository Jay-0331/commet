//! Typed representation of `git status --porcelain=v1 -z` output.
//!
//! Surfaces every entry as a [`FileEntry`] carrying the primary path
//! plus a [`FileStatus`] discriminant. Rename and copy entries carry
//! both the old and new paths so callers can render `old → new`
//! directly. Conflict codes (`UU`, `AA`, `DD`, anything containing
//! `U`) collapse to a single [`FileStatus::Conflicted`] variant;
//! unrecognized codes preserve their two-character marker via
//! [`FileStatus::Other`] so logs stay debuggable.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

use super::wrappers::run_bytes;

/// Status indicator for one tracked or untracked file.
///
/// `git status --porcelain=v1` reports a two-character code per
/// entry. Most callers (file picker, diff filter) only care about
/// the coarse category; renames are the exception because the TUI
/// renders `old → new` and the diff filter must consider both paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// `??` — present on disk, not in the index.
    Untracked,
    /// `A ` or ` A` — newly tracked.
    Added,
    /// `M `, ` M`, `MM`, etc. — content differs.
    Modified,
    /// `D ` or ` D` — removed from the working tree or index.
    Deleted,
    /// `R…` or `C…` — renamed or copied. Carries both paths so
    /// callers can show `from → to` without re-querying git.
    Renamed { from: PathBuf, to: PathBuf },
    /// Any conflict marker — `UU`, `AA`, `DD`, or anything
    /// containing `U`. Resolution UX (#22 doesn't cover it) can
    /// further inspect the index if it needs the exact code.
    Conflicted,
    /// Any code we don't classify explicitly. The two-character
    /// marker is preserved so issue reports / debug logs include it.
    Other(String),
}

/// One row from `git status --porcelain=v1 -z`.
///
/// For renames / copies, [`path`](Self::path) is the **new** path
/// (matching what `git status` puts first); the old path lives
/// inside the [`FileStatus::Renamed`] variant.
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
/// two-character status code, one separator byte, and then the
/// primary path. Rename/copy entries (`R…` / `C…`) are followed by
/// an additional NUL-terminated old path, which the parser consumes
/// to populate [`FileStatus::Renamed::from`].
pub fn parse_porcelain(bytes: &[u8]) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    let mut chunks = bytes.split(|b| *b == 0);

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
        let primary = path_from_bytes(&chunk[3..])?;

        let entry = if is_rename_or_copy(code) {
            let old_bytes = chunks.next().filter(|c| !c.is_empty()).ok_or_else(|| {
                Error::Git(format!(
                    "rename/copy entry missing old path after `{}`",
                    String::from_utf8_lossy(code),
                ))
            })?;
            let from = path_from_bytes(old_bytes)?;
            FileEntry {
                path: primary.clone(),
                status: FileStatus::Renamed { from, to: primary },
            }
        } else {
            FileEntry {
                path: primary,
                status: classify(code),
            }
        };
        entries.push(entry);
    }

    Ok(entries)
}

/// `true` when the two-character code introduces a rename or copy
/// entry — those have a second NUL-terminated path field with the
/// old path that the parser must consume.
fn is_rename_or_copy(code: &[u8]) -> bool {
    code.first().is_some_and(|c| matches!(c, b'R' | b'C'))
}

/// Classify any non-rename, non-copy code.
///
/// The set of "modified-like" codes is enumerated explicitly so that
/// unfamiliar markers (e.g. `!!` for ignored files, future git
/// additions) fall through to [`FileStatus::Other`] with their raw
/// code preserved, rather than being silently misclassified.
fn classify(code: &[u8]) -> FileStatus {
    match code {
        b"??" => FileStatus::Untracked,
        b"A " | b" A" => FileStatus::Added,
        b"D " | b" D" => FileStatus::Deleted,
        // Modified-like codes: M/T in either column (with optional
        // pairing) and AM (added in index, modified in worktree).
        b"M " | b" M" | b"MM" | b"AM" | b"MD" | b"MT" | b"TM" | b"T " | b" T" | b"TT" => {
            FileStatus::Modified
        }
        // Conflict matrix: every `U` combination plus the two
        // both-sides-modified codes (`AA`, `DD`) that don't contain U.
        c if c.contains(&b'U') || c == b"AA" || c == b"DD" => FileStatus::Conflicted,
        _ => FileStatus::Other(String::from_utf8_lossy(code).into_owned()),
    }
}

fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf> {
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
        let entries = parse_porcelain(b"?? notes.md\0").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("notes.md"));
        assert_eq!(entries[0].status, FileStatus::Untracked);
    }

    #[test]
    fn single_modified_entry() {
        let entries = parse_porcelain(b" M src/main.rs\0").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("src/main.rs"));
        assert_eq!(entries[0].status, FileStatus::Modified);
    }

    #[test]
    fn single_added_entry() {
        let entries = parse_porcelain(b"A  src/new.rs\0").unwrap();
        assert_eq!(entries[0].status, FileStatus::Added);
    }

    #[test]
    fn am_pair_is_modified() {
        // `AM` = added-then-modified; treat as modified (added is
        // already covered by `A `).
        let entries = parse_porcelain(b"AM src/main.rs\0").unwrap();
        assert_eq!(entries[0].status, FileStatus::Modified);
    }

    #[test]
    fn single_deleted_entry() {
        let entries = parse_porcelain(b" D src/gone.rs\0").unwrap();
        assert_eq!(entries[0].status, FileStatus::Deleted);
    }

    #[test]
    fn conflict_codes_classified_as_conflicted() {
        // The full unmerged matrix from `git status` docs.
        for code in [&b"UU"[..], b"AA", b"DD", b"AU", b"UA", b"UD", b"DU"] {
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
    fn rename_entry_keeps_both_paths() {
        // Format: "R  <new>\0<old>\0"
        let entries = parse_porcelain(b"R  new.rs\0old.rs\0").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("new.rs"));
        match &entries[0].status {
            FileStatus::Renamed { from, to } => {
                assert_eq!(from, &PathBuf::from("old.rs"));
                assert_eq!(to, &PathBuf::from("new.rs"));
            }
            other => panic!("expected Renamed, got {other:?}"),
        }
    }

    #[test]
    fn copy_entry_keeps_both_paths() {
        let entries = parse_porcelain(b"C  copy.rs\0orig.rs\0").unwrap();
        assert_eq!(entries.len(), 1);
        match &entries[0].status {
            FileStatus::Renamed { from, to } => {
                assert_eq!(from, &PathBuf::from("orig.rs"));
                assert_eq!(to, &PathBuf::from("copy.rs"));
            }
            other => panic!("expected Renamed (for copy), got {other:?}"),
        }
    }

    #[test]
    fn renamed_with_modified_worktree_is_still_renamed() {
        // `RM` = renamed-in-index, modified-in-worktree.
        let entries = parse_porcelain(b"RM after.rs\0before.rs\0").unwrap();
        match &entries[0].status {
            FileStatus::Renamed { from, to } => {
                assert_eq!(from, &PathBuf::from("before.rs"));
                assert_eq!(to, &PathBuf::from("after.rs"));
            }
            other => panic!("expected Renamed, got {other:?}"),
        }
    }

    #[test]
    fn rename_missing_old_path_errors_clearly() {
        // Single chunk, no second NUL-terminated field.
        let err = parse_porcelain(b"R  new.rs\0").unwrap_err();
        assert!(
            matches!(&err, Error::Git(msg) if msg.contains("missing old path")),
            "expected missing-old-path error, got: {err:?}",
        );
    }

    #[test]
    fn other_variant_preserves_raw_code() {
        // `!!` (ignored) isn't classified specifically; surface as Other("!!").
        let entries = parse_porcelain(b"!! ignored.bin\0").unwrap();
        match &entries[0].status {
            FileStatus::Other(code) => assert_eq!(code, "!!"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn multiple_mixed_entries_including_a_rename() {
        let bytes = b" M src/a.rs\0?? notes.md\0R  to.rs\0from.rs\0A  src/b.rs\0 D removed.rs\0";
        let entries = parse_porcelain(bytes).unwrap();
        assert_eq!(entries.len(), 5);

        // Compare against a vector of `&FileStatus` (no Copy required).
        let statuses: Vec<&FileStatus> = entries.iter().map(|e| &e.status).collect();
        assert_eq!(statuses[0], &FileStatus::Modified);
        assert_eq!(statuses[1], &FileStatus::Untracked);
        assert!(matches!(
            statuses[2],
            FileStatus::Renamed { from, to }
                if from == &PathBuf::from("from.rs") && to == &PathBuf::from("to.rs"),
        ));
        assert_eq!(statuses[3], &FileStatus::Added);
        assert_eq!(statuses[4], &FileStatus::Deleted);
    }

    #[test]
    fn paths_with_spaces_are_preserved() {
        let entries = parse_porcelain(b" M docs/notes for ops.md\0").unwrap();
        assert_eq!(entries[0].path, PathBuf::from("docs/notes for ops.md"));
    }

    #[test]
    fn too_short_chunk_errors_clearly() {
        let err = parse_porcelain(b"??\0").unwrap_err();
        assert!(matches!(err, Error::Git(msg) if msg.contains("too short")));
    }

    #[test]
    fn non_utf8_path_errors_clearly() {
        // 0xFF is invalid UTF-8 in any position.
        let bytes = b"?? \xff\xff\0";
        let err = parse_porcelain(bytes).unwrap_err();
        assert!(
            matches!(err, Error::Git(msg) if msg.contains("not utf-8")),
            "expected utf-8 error",
        );
    }

    #[test]
    fn is_rename_or_copy_matches_r_and_c_prefixes() {
        assert!(is_rename_or_copy(b"R "));
        assert!(is_rename_or_copy(b"RM"));
        assert!(is_rename_or_copy(b"C "));
        assert!(!is_rename_or_copy(b"M "));
        assert!(!is_rename_or_copy(b"??"));
        assert!(!is_rename_or_copy(b""));
    }
}
