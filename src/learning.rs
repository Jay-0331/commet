//! Append-only JSONL store for accepted commit messages.
//!
//! Each accepted commit appends one [`LearningRecord`] as a single JSON
//! line ([`append`]). Later runs read the store back ([`load`]) to feed
//! few-shot examples into the prompt (#45). The store lives either
//! globally (`$XDG_STATE_HOME/commitcrafter/history.jsonl`) or per-repo
//! (`<repo>/.commitcrafter/history.jsonl`); scope selection is #46.
//!
//! When the live file grows past [`MAX_BYTES`] it rotates to numbered
//! archives (`.1`…`.3`, oldest dropped). Only the live file feeds
//! future prompts — archives are kept for `cc history` but never loaded
//! into a prompt.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Store filename under the commitcrafter state/repo directory.
const STORE_FILE: &str = "history.jsonl";

/// Rotate once the live file exceeds 5 MiB.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Number of rotated archives to keep (`.1`, `.2`, `.3`).
const KEEP_ARCHIVES: u32 = 3;

/// One learning entry: everything needed to replay an accepted commit
/// as a future few-shot example, plus provenance for `cc history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningRecord {
    /// ISO-8601 timestamp, supplied by the caller (keeps this module
    /// free of a clock dependency and deterministic in tests).
    pub ts: String,
    pub repo: String,
    pub branch: String,
    pub provider: String,
    pub model: String,
    pub format: String,
    /// All candidate messages generated this run.
    pub candidates: Vec<String>,
    /// Index into `candidates` that the user accepted.
    pub accepted_index: usize,
    /// Final message text after any manual edit.
    pub edited_text: String,
    /// Files included in the diff.
    pub files: Vec<String>,
    /// Size of the (possibly shrunk) diff fed to the model.
    pub diff_bytes: usize,
    /// The diff text itself.
    pub diff: String,
}

/// Global store path: `$XDG_STATE_HOME/commitcrafter/history.jsonl`,
/// falling back to `$HOME/.local/state/commitcrafter/history.jsonl`.
/// `None` when neither environment variable is usable.
pub fn global_store_path() -> Option<PathBuf> {
    let xdg = std::env::var("XDG_STATE_HOME").ok();
    let home = std::env::var("HOME").ok();
    global_store_path_with(xdg.as_deref(), home.as_deref())
}

/// Pure form of [`global_store_path`] for testing.
pub fn global_store_path_with(xdg_state_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(xdg) = xdg_state_home
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join("commitcrafter").join(STORE_FILE));
    }
    let home = home?;
    if home.is_empty() {
        return None;
    }
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("commitcrafter")
            .join(STORE_FILE),
    )
}

/// Per-repo store path: `<repo_root>/.commitcrafter/history.jsonl`.
pub fn repo_store_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".commitcrafter").join(STORE_FILE)
}

/// Append one record as a JSON line to `path`, creating parent
/// directories and rotating first if the file is over [`MAX_BYTES`].
pub fn append(path: &Path, record: &LearningRecord) -> Result<()> {
    append_with_limit(path, record, MAX_BYTES)
}

/// [`append`] with an explicit rotation threshold (so tests don't have
/// to write 5 MiB).
fn append_with_limit(path: &Path, record: &LearningRecord, max_bytes: u64) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    rotate_if_needed(path, max_bytes)?;

    let line = serde_json::to_string(record)
        .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Read every record from the **live** store file (archives excluded).
/// Missing file → empty vec. Malformed lines are skipped with a warning
/// so one corrupt entry can't poison history.
pub fn load(path: &Path) -> Result<Vec<LearningRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let mut records = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<LearningRecord>(line) {
            Ok(record) => records.push(record),
            Err(e) => tracing::warn!(error = %e, "skipping malformed history line"),
        }
    }
    Ok(records)
}

/// If `path` is over `max_bytes`, shift archives (`.2`→`.3`, `.1`→`.2`,
/// dropping the old `.3`) and move the live file to `.1`. The next
/// [`append`] recreates the live file.
fn rotate_if_needed(path: &Path, max_bytes: u64) -> Result<()> {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size <= max_bytes {
        return Ok(());
    }

    for n in (1..KEEP_ARCHIVES).rev() {
        let src = archive_path(path, n);
        if src.exists() {
            fs::rename(&src, archive_path(path, n + 1))?;
        }
    }
    fs::rename(path, archive_path(path, 1))?;
    Ok(())
}

/// The `path.N` archive name (e.g. `history.jsonl.1`).
fn archive_path(base: &Path, n: u32) -> PathBuf {
    let mut name = base.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn record(text: &str) -> LearningRecord {
        LearningRecord {
            ts: "2026-07-11T00:00:00Z".into(),
            repo: "commitcrafter".into(),
            branch: "main".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4-6".into(),
            format: "conventional".into(),
            candidates: vec![text.into()],
            accepted_index: 0,
            edited_text: text.into(),
            files: vec!["src/lib.rs".into()],
            diff_bytes: 42,
            diff: "diff --git ...".into(),
        }
    }

    #[test]
    fn append_then_load_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");

        append(&path, &record("feat: one")).unwrap();
        append(&path, &record("fix: two")).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].edited_text, "feat: one");
        assert_eq!(loaded[1].edited_text, "fix: two");
        assert_eq!(loaded[0], record("feat: one"));
    }

    #[test]
    fn load_missing_file_is_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.jsonl");
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn append_creates_parent_directories() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".commitcrafter").join("history.jsonl");
        append(&path, &record("feat: nested")).unwrap();
        assert!(path.exists());
        assert_eq!(load(&path).unwrap().len(), 1);
    }

    #[test]
    fn load_skips_malformed_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        append(&path, &record("feat: good")).unwrap();
        // Append a junk line by hand.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{ not valid json").unwrap();
        append(&path, &record("fix: also good")).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn rotates_when_over_threshold() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");

        // First write with a tiny threshold: file is empty (0 bytes), no
        // rotation, so the record lands in the live file.
        append_with_limit(&path, &record("feat: first"), 10).unwrap();
        assert!(!archive_path(&path, 1).exists());

        // Second write: live file now exceeds 10 bytes, so it rotates to
        // `.1` and the new record starts a fresh live file.
        append_with_limit(&path, &record("fix: second"), 10).unwrap();
        assert!(archive_path(&path, 1).exists());

        let live = load(&path).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].edited_text, "fix: second");

        let archived = load(&archive_path(&path, 1)).unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].edited_text, "feat: first");
    }

    #[test]
    fn rotation_keeps_three_archives_and_drops_oldest() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");

        // Five rotating writes: live, then archives .1 .2 .3 fill up and
        // the oldest is dropped.
        for i in 0..5 {
            append_with_limit(&path, &record(&format!("msg {i}")), 10).unwrap();
        }

        assert!(archive_path(&path, 1).exists());
        assert!(archive_path(&path, 2).exists());
        assert!(archive_path(&path, 3).exists());
        assert!(!archive_path(&path, 4).exists());

        // Live holds the newest; .1/.2/.3 hold successively older ones.
        assert_eq!(load(&path).unwrap()[0].edited_text, "msg 4");
        assert_eq!(
            load(&archive_path(&path, 1)).unwrap()[0].edited_text,
            "msg 3"
        );
        assert_eq!(
            load(&archive_path(&path, 3)).unwrap()[0].edited_text,
            "msg 1"
        );
    }

    #[test]
    fn load_reads_only_the_live_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        append_with_limit(&path, &record("feat: live-1"), 10).unwrap();
        append_with_limit(&path, &record("feat: live-2"), 10).unwrap(); // rotates live-1 to .1

        // load(live) must not surface the archived record.
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].edited_text, "feat: live-2");
    }

    #[test]
    fn global_path_prefers_xdg_state_home() {
        let p = global_store_path_with(Some("/xdg/state"), Some("/home/u")).unwrap();
        assert_eq!(p, PathBuf::from("/xdg/state/commitcrafter/history.jsonl"));
    }

    #[test]
    fn global_path_falls_back_to_home_local_state() {
        let p = global_store_path_with(None, Some("/home/u")).unwrap();
        assert_eq!(
            p,
            PathBuf::from("/home/u/.local/state/commitcrafter/history.jsonl")
        );
        assert!(global_store_path_with(Some(""), None).is_none());
    }

    #[test]
    fn repo_path_is_under_dot_commitcrafter() {
        let p = repo_store_path(Path::new("/work/repo"));
        assert_eq!(p, PathBuf::from("/work/repo/.commitcrafter/history.jsonl"));
    }
}
