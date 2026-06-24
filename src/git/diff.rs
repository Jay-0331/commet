//! Diff size cap + truncation strategy.
//!
//! Large diffs blow the LLM's context budget. This module
//! implements a deterministic three-stage shrinking pipeline:
//!
//! 1. **Drop ignored paths first.** Any per-file chunk whose path
//!    matches one of the configured ignore globs is removed
//!    outright — `package-lock.json`, `*.lock`, `dist/**`, and any
//!    user-added patterns from `[git].ignore_paths`.
//! 2. **Drop the largest remaining files.** If the diff is still
//!    above the byte cap, files are dropped in descending order of
//!    size until the remainder fits.
//! 3. **Truncate the tail with a marker.** If after step 2 the
//!    remaining text is still too big (one huge file that survived
//!    both filters), the trailing bytes are cut and replaced with
//!    `... <N> bytes truncated across <M> files ...` so the LLM
//!    sees the truncation explicitly.
//!
//! A short header (`# N files changed: ...`) is always preserved
//! and prepended to the output so even an empty diff still tells
//! the model what the user staged.
//!
//! This module is **pure**: it does no I/O. Callers feed in the raw
//! `git diff --cached` output plus the porcelain status list they
//! already obtained from [`crate::git::status_porcelain`].

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::error::{Error, Result};

use super::status::{FileEntry, FileStatus};

/// One file's portion of a unified diff (the `diff --git` block
/// plus everything up to the next `diff --git`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffChunk {
    /// Path extracted from the `b/...` side of the diff header.
    /// For renames this is the destination path.
    pub path: String,
    /// The raw chunk text — `diff --git a/X b/Y\n...` through the
    /// last line before the next chunk.
    pub text: String,
}

impl DiffChunk {
    pub fn byte_len(&self) -> usize {
        self.text.len()
    }
}

/// Split a unified diff into per-file [`DiffChunk`]s.
///
/// Splits on the `diff --git ` line, which is the canonical
/// per-file boundary in `git diff` output. Empty input returns an
/// empty vector.
pub fn parse_chunks(diff: &str) -> Vec<DiffChunk> {
    let mut chunks = Vec::new();
    let mut current_header: Option<&str> = None;
    let mut current_start: usize = 0;

    let mut byte_idx = 0usize;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            if let Some(header) = current_header.take() {
                let end = byte_idx;
                let text = &diff[current_start..end];
                chunks.push(make_chunk(header, text));
            }
            current_header = Some(line);
            current_start = byte_idx;
        }
        byte_idx += line.len();
    }
    if let Some(header) = current_header {
        let text = &diff[current_start..];
        chunks.push(make_chunk(header, text));
    }

    chunks
}

fn make_chunk(header_line: &str, text: &str) -> DiffChunk {
    DiffChunk {
        path: parse_path_from_header(header_line),
        text: text.to_string(),
    }
}

/// Pull the destination path out of `diff --git a/foo b/bar`.
fn parse_path_from_header(header: &str) -> String {
    // Header is a single line ending in '\n'. After "diff --git "
    // come two space-separated paths; the second one (b/...) is
    // what we want. Paths can contain spaces, but git quotes them
    // with `"..."` when they do — the basic best-effort here covers
    // every non-quoted path; we keep the raw header path otherwise.
    let trimmed = header.trim_end_matches(['\n', '\r']);
    let rest = trimmed.strip_prefix("diff --git ").unwrap_or(trimmed);
    if let Some(b_idx) = rest.rfind(" b/") {
        let b_path = &rest[b_idx + 3..];
        b_path.trim_matches('"').to_string()
    } else if let Some(b_idx) = rest.rfind(" \"b/") {
        rest[b_idx + 4..].trim_end_matches('"').to_string()
    } else {
        rest.to_string()
    }
}

/// Build a one-line summary like
/// `# 5 files changed: 2 modified, 1 added, 1 deleted, 1 renamed`
/// suitable for prepending to any rendered diff. Always non-empty
/// (even for zero entries it emits `# 0 files changed`).
pub fn header_summary(entries: &[FileEntry]) -> String {
    let mut counts = [0usize; 6];
    for entry in entries {
        let bucket = match entry.status {
            FileStatus::Modified => 0,
            FileStatus::Added => 1,
            FileStatus::Deleted => 2,
            FileStatus::Renamed { .. } => 3,
            FileStatus::Untracked => 4,
            FileStatus::Conflicted | FileStatus::Other(_) => 5,
        };
        counts[bucket] += 1;
    }

    let parts = [
        ("modified", counts[0]),
        ("added", counts[1]),
        ("deleted", counts[2]),
        ("renamed", counts[3]),
        ("untracked", counts[4]),
        ("other", counts[5]),
    ];
    let pieces: Vec<String> = parts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(label, n)| format!("{n} {label}"))
        .collect();

    let total = entries.len();
    if pieces.is_empty() {
        format!("# {total} files changed")
    } else {
        format!("# {total} files changed: {}", pieces.join(", "))
    }
}

/// The marker line appended when the trailing-bytes truncation
/// stage fires.
fn truncation_marker(byte_count: usize, file_count: usize) -> String {
    format!("\n... {byte_count} bytes truncated across {file_count} files ...\n",)
}

/// Build a `GlobSet` from a list of pattern strings.
fn build_globs(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern)
            .map_err(|e| Error::Git(format!("invalid ignore_paths glob `{pattern}`: {e}")))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| Error::Git(format!("failed to compile ignore_paths globset: {e}")))
}

/// Run the full truncation pipeline.
///
/// Returns the (possibly shrunken) diff string with the header
/// summary prepended. The total length of the returned string is
/// not guaranteed to be ≤ `max_bytes` — only the *diff body* after
/// the header is bounded. This matches what the LLM cares about
/// (it sees both, and the header is tiny).
pub fn truncate(
    diff: &str,
    entries: &[FileEntry],
    max_bytes: usize,
    ignore_globs: &[String],
) -> Result<String> {
    let header = header_summary(entries);

    let mut chunks = parse_chunks(diff);
    let globs = build_globs(ignore_globs)?;

    let initial_total: usize = chunks.iter().map(DiffChunk::byte_len).sum();

    // Stage 1: drop ignored paths.
    let mut dropped_files = 0usize;
    let mut dropped_bytes = 0usize;
    chunks.retain(|c| {
        if globs.is_match(&c.path) {
            dropped_files += 1;
            dropped_bytes += c.byte_len();
            false
        } else {
            true
        }
    });

    // Stage 2: drop largest until under budget. Always keep at
    // least one chunk so stage 3 can truncate its tail rather than
    // wiping the diff entirely (otherwise a single huge file would
    // disappear without trace).
    let mut remaining: usize = chunks.iter().map(DiffChunk::byte_len).sum();
    while remaining > max_bytes && chunks.len() > 1 {
        let largest_idx = chunks
            .iter()
            .enumerate()
            .max_by_key(|(_, c)| c.byte_len())
            .map(|(i, _)| i)
            .expect("non-empty by while-guard");
        dropped_files += 1;
        let removed = chunks.remove(largest_idx);
        dropped_bytes += removed.byte_len();
        remaining = remaining.saturating_sub(removed.byte_len());
    }

    // Stage 3: if the surviving chunks are still over budget (one
    // huge file survived both filters), truncate the joined text.
    let joined: String = chunks.iter().map(|c| c.text.as_str()).collect();
    let body = if joined.len() > max_bytes {
        let cut_at = byte_floor_char_boundary(&joined, max_bytes);
        let trailing_bytes = joined.len() - cut_at;
        let marker = truncation_marker(trailing_bytes + dropped_bytes, dropped_files + 1);
        format!("{}{}", &joined[..cut_at], marker)
    } else if dropped_files > 0 {
        let marker = truncation_marker(dropped_bytes, dropped_files);
        format!("{joined}{marker}")
    } else {
        joined
    };

    tracing::debug!(
        initial_bytes = initial_total,
        kept_bytes = body.len(),
        cap = max_bytes,
        ignore_glob_count = ignore_globs.len(),
        "diff truncation",
    );

    Ok(format!("{header}\n{body}"))
}

/// Find the largest index ≤ `cap` that lands on a UTF-8 character
/// boundary inside `s`. Avoids slicing in the middle of a
/// multi-byte rune.
fn byte_floor_char_boundary(s: &str, cap: usize) -> usize {
    let cap = cap.min(s.len());
    let mut idx = cap;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn modified(path: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            status: FileStatus::Modified,
        }
    }

    fn added(path: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            status: FileStatus::Added,
        }
    }

    fn deleted(path: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            status: FileStatus::Deleted,
        }
    }

    fn diff_for(path: &str, body_size: usize) -> String {
        let header = format!(
            "diff --git a/{path} b/{path}\nindex 0000..1111 100644\n--- a/{path}\n+++ b/{path}\n",
        );
        let body: String = "+".repeat(body_size);
        format!("{header}{body}\n")
    }

    #[test]
    fn parse_chunks_empty_input_returns_empty() {
        assert!(parse_chunks("").is_empty());
    }

    #[test]
    fn parse_chunks_recovers_path_from_b_side() {
        let diff = diff_for("src/main.rs", 0) + &diff_for("notes.md", 0);
        let chunks = parse_chunks(&diff);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].path, "src/main.rs");
        assert_eq!(chunks[1].path, "notes.md");
    }

    #[test]
    fn parse_chunks_preserves_chunk_text() {
        let diff = diff_for("a.rs", 5) + &diff_for("b.rs", 7);
        let chunks = parse_chunks(&diff);
        assert!(chunks[0].text.contains("a/a.rs"));
        assert!(chunks[1].text.contains("a/b.rs"));
        // Concatenating the chunks reproduces the input.
        let joined: String = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(joined, diff);
    }

    #[test]
    fn header_summary_empty_entries() {
        assert_eq!(header_summary(&[]), "# 0 files changed");
    }

    #[test]
    fn header_summary_mixed_counts() {
        let entries = vec![
            modified("a.rs"),
            modified("b.rs"),
            added("c.rs"),
            deleted("d.rs"),
        ];
        assert_eq!(
            header_summary(&entries),
            "# 4 files changed: 2 modified, 1 added, 1 deleted",
        );
    }

    #[test]
    fn truncate_below_cap_returns_full_diff_with_header() {
        let entries = vec![modified("a.rs")];
        let diff = diff_for("a.rs", 100);
        let out = truncate(&diff, &entries, 10_000, &[]).unwrap();
        assert!(out.starts_with("# 1 files changed"));
        assert!(out.contains("a/a.rs"));
        assert!(!out.contains("truncated"));
    }

    #[test]
    fn truncate_drops_ignored_paths_first() {
        let entries = vec![modified("package-lock.json"), modified("src/main.rs")];
        let diff = diff_for("package-lock.json", 150_000) + &diff_for("src/main.rs", 200);
        let ignore = vec!["package-lock.json".to_string()];

        let out = truncate(&diff, &entries, 100 * 1024, &ignore).unwrap();
        // lockfile gone, real source preserved
        assert!(
            !out.contains("a/package-lock.json"),
            "lockfile was not dropped:\n{out}",
        );
        assert!(out.contains("a/src/main.rs"), "real source dropped:\n{out}");
        assert!(
            out.contains("truncated across 1 files"),
            "expected truncation marker; got:\n{out}",
        );
    }

    #[test]
    fn truncate_drops_largest_remaining_after_ignored() {
        let entries = vec![
            modified("package-lock.json"),
            modified("huge.rs"),
            modified("small.rs"),
        ];
        // Total ~ 250 KB; cap 100 KB. Lockfile drop alone won't fit.
        let diff = diff_for("package-lock.json", 100_000)
            + &diff_for("huge.rs", 150_000)
            + &diff_for("small.rs", 200);
        let ignore = vec!["package-lock.json".to_string()];

        let out = truncate(&diff, &entries, 100 * 1024, &ignore).unwrap();
        assert!(!out.contains("a/package-lock.json"));
        assert!(!out.contains("a/huge.rs"), "huge file should be dropped");
        assert!(out.contains("a/small.rs"), "small file should survive");
        assert!(out.contains("truncated across 2 files"));
    }

    #[test]
    fn truncate_text_when_one_remaining_file_still_over_cap() {
        let entries = vec![added("giant.rs")];
        // One file, larger than cap, nothing to drop. Tail must be
        // chopped with a marker.
        let diff = diff_for("giant.rs", 200_000);
        let out = truncate(&diff, &entries, 50_000, &[]).unwrap();
        assert!(out.contains("a/giant.rs"));
        assert!(out.contains("truncated"));
        // Body length should be near the cap (header is small).
        let body_len = out
            .find("# 1 files changed")
            .map(|_| out.len())
            .unwrap_or(0);
        assert!(body_len < 200_000, "did not actually shrink: {body_len}");
    }

    #[test]
    fn truncate_zero_chunks_still_emits_header() {
        let out = truncate("", &[], 100, &[]).unwrap();
        assert!(out.starts_with("# 0 files changed"));
    }

    #[test]
    fn truncate_invalid_glob_surfaces_error() {
        let entries = vec![modified("a.rs")];
        let diff = diff_for("a.rs", 10);
        // `[invalid` is an unterminated character class.
        let err = truncate(&diff, &entries, 1000, &["[invalid".to_string()]).unwrap_err();
        assert!(matches!(err, Error::Git(msg) if msg.contains("invalid ignore_paths glob")));
    }

    #[test]
    fn glob_supports_lock_wildcard() {
        let entries = vec![
            modified("Cargo.lock"),
            modified("yarn.lock"),
            modified("src/main.rs"),
        ];
        let diff = diff_for("Cargo.lock", 60_000)
            + &diff_for("yarn.lock", 60_000)
            + &diff_for("src/main.rs", 200);
        let ignore = vec!["*.lock".to_string()];

        let out = truncate(&diff, &entries, 100 * 1024, &ignore).unwrap();
        assert!(!out.contains("a/Cargo.lock"));
        assert!(!out.contains("a/yarn.lock"));
        assert!(out.contains("a/src/main.rs"));
        assert!(out.contains("truncated across 2 files"));
    }

    #[test]
    fn glob_supports_double_star_directory() {
        let entries = vec![modified("dist/bundle.js"), modified("src/main.rs")];
        let diff = diff_for("dist/bundle.js", 50_000) + &diff_for("src/main.rs", 100);
        let ignore = vec!["dist/**".to_string()];

        let out = truncate(&diff, &entries, 100 * 1024, &ignore).unwrap();
        assert!(!out.contains("dist/bundle.js"));
        assert!(out.contains("src/main.rs"));
    }

    #[test]
    fn byte_floor_char_boundary_avoids_mid_char_slicing() {
        let s = "héllo"; // 'é' is two bytes
        // Asking for cap = 2 lands in the middle of 'é'; should
        // round down to 1.
        let cap = byte_floor_char_boundary(s, 2);
        assert!(s.is_char_boundary(cap));
        assert_eq!(&s[..cap], "h");
    }
}
