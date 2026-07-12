//! Append-only JSONL store for accepted commit messages.
//!
//! Each accepted commit appends one [`LearningRecord`] as a single JSON
//! line ([`append`]). Later runs read the store back ([`load`]) to feed
//! few-shot examples into the prompt (#45). The store lives either
//! globally (`$XDG_STATE_HOME/commet/history.jsonl`) or per-repo
//! (`<repo>/.commet/history.jsonl`); scope selection is #46.
//!
//! When the live file grows past [`MAX_BYTES`] it rotates to numbered
//! archives (`.1`…`.3`, oldest dropped). Only the live file feeds
//! future prompts — archives are kept for `cc history` but never loaded
//! into a prompt.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::schema::LearningScope;
use crate::error::{Error, Result};

/// Store filename under the commet state/repo directory.
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

/// Global store path: `$XDG_STATE_HOME/commet/history.jsonl`,
/// falling back to `$HOME/.local/state/commet/history.jsonl`.
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
        return Some(PathBuf::from(xdg).join("commet").join(STORE_FILE));
    }
    let home = home?;
    if home.is_empty() {
        return None;
    }
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("commet")
            .join(STORE_FILE),
    )
}

/// Per-repo store directory, and the line written to `.gitignore`.
const REPO_STORE_DIR: &str = ".commet";
const GITIGNORE_ENTRY: &str = ".commet/";

/// Per-repo store path: `<repo_root>/.commet/history.jsonl`.
pub fn repo_store_path(repo_root: &Path) -> PathBuf {
    repo_root.join(REPO_STORE_DIR).join(STORE_FILE)
}

/// Ensure the per-repo store dir is excluded by the repo's `.gitignore`
/// so history never gets committed. Creates `.gitignore` if absent,
/// appends the entry (with a comment) if not already present, no-op when
/// it exists. Skips silently when `repo_root` has no `.git`. Returns
/// whether an entry was added.
pub fn ensure_gitignored(repo_root: &Path) -> Result<bool> {
    if !repo_root.join(".git").exists() {
        return Ok(false); // not a working tree (bare / missing) — nothing to do
    }

    let gitignore = repo_root.join(".gitignore");
    let existing = fs::read_to_string(&gitignore).unwrap_or_default();
    if existing
        .lines()
        .any(|l| matches!(l.trim(), GITIGNORE_ENTRY | REPO_STORE_DIR))
    {
        return Ok(false);
    }

    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\n# commet: per-repo commit-message history (do not commit)\n");
    out.push_str(GITIGNORE_ENTRY);
    out.push('\n');
    fs::write(&gitignore, out)?;
    Ok(true)
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

/// Delete a store file and all of its rotated archives (`.1`..`.3`).
/// Returns whether anything was removed (for "nothing to forget"
/// detection). Full wipe, so archived history is cleared too.
pub fn clear(path: &Path) -> Result<bool> {
    let mut removed = false;
    for candidate in std::iter::once(path.to_path_buf())
        .chain((1..=KEEP_ARCHIVES).map(|n| archive_path(path, n)))
    {
        if candidate.exists() {
            fs::remove_file(&candidate)?;
            removed = true;
        }
    }
    Ok(removed)
}

/// Overwrite `path` with exactly `records` (one JSON line each), or
/// delete it when `records` is empty.
fn rewrite(path: &Path, records: &[LearningRecord]) -> Result<()> {
    if records.is_empty() {
        clear(path)?;
        return Ok(());
    }
    let mut out = String::new();
    for record in records {
        let line = serde_json::to_string(record)
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        out.push_str(&line);
        out.push('\n');
    }
    fs::write(path, out)?;
    Ok(())
}

/// Drop the single most-recent record across `paths` (by timestamp,
/// ties → last appended), rewriting the file it lived in. Returns
/// whether a record was removed.
pub fn drop_last(paths: &[PathBuf]) -> Result<bool> {
    let loaded: Vec<Vec<LearningRecord>> = paths.iter().map(|p| load(p)).collect::<Result<_>>()?;

    let mut best: Option<(usize, usize)> = None; // (file, record)
    for (pi, records) in loaded.iter().enumerate() {
        for (ri, record) in records.iter().enumerate() {
            let newer = best
                .map(|(bp, br)| record.ts >= loaded[bp][br].ts)
                .unwrap_or(true);
            if newer {
                best = Some((pi, ri));
            }
        }
    }

    let Some((pi, ri)) = best else {
        return Ok(false);
    };
    let mut records = loaded[pi].clone();
    records.remove(ri);
    rewrite(&paths[pi], &records)?;
    Ok(true)
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

/// The learning store as seen through a configured [`LearningScope`].
///
/// A `Store` resolves the repo and global store paths once, then gates
/// every read/write by scope: `Off` touches nothing, `Repo`/`Global`
/// touch one file, and `RepoGlobal` touches both (repo entries ranked
/// first on read, both written on append).
pub struct Store {
    scope: LearningScope,
    repo_path: Option<PathBuf>,
    global_path: Option<PathBuf>,
}

impl Store {
    /// Resolve store paths for `scope`. `repo_root` is the repository
    /// root (`None` when not inside a repo); the global path comes from
    /// the environment via [`global_store_path`].
    pub fn open(scope: LearningScope, repo_root: Option<&Path>) -> Self {
        Self::with_paths(scope, repo_root.map(repo_store_path), global_store_path())
    }

    /// Construct a store from explicit paths — the injection point for
    /// tests and callers that resolve paths themselves.
    pub fn with_paths(
        scope: LearningScope,
        repo_path: Option<PathBuf>,
        global_path: Option<PathBuf>,
    ) -> Self {
        Self {
            scope,
            repo_path,
            global_path,
        }
    }

    /// Whether the scope enables the store at all (`scope != off`).
    pub fn is_enabled(&self) -> bool {
        !matches!(self.scope, LearningScope::Off)
    }

    /// The files this scope touches, as owned paths (repo before
    /// global). Useful for user-facing hints (e.g. "looked in …").
    pub fn paths(&self) -> Vec<PathBuf> {
        self.active_paths()
            .into_iter()
            .map(Path::to_path_buf)
            .collect()
    }

    /// The files this scope touches, repo before global. `Off` yields
    /// none; a scope naming a file whose path is unresolved (e.g. `repo`
    /// outside a repo) simply drops it.
    fn active_paths(&self) -> Vec<&Path> {
        let use_repo = matches!(self.scope, LearningScope::Repo | LearningScope::RepoGlobal);
        let use_global = matches!(
            self.scope,
            LearningScope::Global | LearningScope::RepoGlobal
        );

        let mut paths = Vec::new();
        if use_repo && let Some(p) = &self.repo_path {
            paths.push(p.as_path());
        }
        if use_global && let Some(p) = &self.global_path {
            paths.push(p.as_path());
        }
        paths
    }

    /// Load every in-scope record, repo entries first. `Off` → empty.
    pub fn read(&self) -> Result<Vec<LearningRecord>> {
        let mut records = Vec::new();
        for path in self.active_paths() {
            records.extend(load(path)?);
        }
        Ok(records)
    }

    /// Append `record` to every in-scope file. `Off` → no-op.
    pub fn write(&self, record: &LearningRecord) -> Result<()> {
        for path in self.active_paths() {
            append(path, record)?;
        }
        Ok(())
    }

    /// Up to `max` most-recent accepted messages matching `format`,
    /// de-duplicated by text — few-shot examples for the prompt.
    pub fn load_examples(&self, format: &str, max: usize) -> Result<Vec<String>> {
        Ok(select_examples(&self.read()?, format, max))
    }
}

/// From oldest→newest `records`, take the `max` most recent accepted
/// messages whose `format` matches, de-duplicated by `edited_text`
/// (keeping the newest occurrence). Returned newest-first.
pub fn select_examples(records: &[LearningRecord], format: &str, max: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for record in records.iter().rev() {
        if record.format != format {
            continue;
        }
        if seen.insert(record.edited_text.as_str()) {
            out.push(record.edited_text.clone());
            if out.len() == max {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn record(text: &str) -> LearningRecord {
        LearningRecord {
            ts: "2026-07-11T00:00:00Z".into(),
            repo: "commet".into(),
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
        let path = dir.path().join(".commet").join("history.jsonl");
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
        assert_eq!(p, PathBuf::from("/xdg/state/commet/history.jsonl"));
    }

    #[test]
    fn global_path_falls_back_to_home_local_state() {
        let p = global_store_path_with(None, Some("/home/u")).unwrap();
        assert_eq!(
            p,
            PathBuf::from("/home/u/.local/state/commet/history.jsonl")
        );
        assert!(global_store_path_with(Some(""), None).is_none());
    }

    /// Make `dir` look like a git working tree.
    fn git_init(dir: &std::path::Path) {
        fs::create_dir_all(dir.join(".git")).unwrap();
    }

    #[test]
    fn ensure_gitignored_creates_file_when_missing() {
        let d = tempdir().unwrap();
        git_init(d.path());
        assert!(ensure_gitignored(d.path()).unwrap());
        let gi = fs::read_to_string(d.path().join(".gitignore")).unwrap();
        assert!(gi.contains(".commet/"));
        assert!(gi.contains("# commet:"));
    }

    #[test]
    fn ensure_gitignored_no_duplicate_when_present() {
        let d = tempdir().unwrap();
        git_init(d.path());
        fs::write(d.path().join(".gitignore"), "target/\n.commet/\n").unwrap();
        assert!(!ensure_gitignored(d.path()).unwrap());
        let gi = fs::read_to_string(d.path().join(".gitignore")).unwrap();
        assert_eq!(gi.matches(".commet/").count(), 1);
    }

    #[test]
    fn ensure_gitignored_appends_to_existing_file() {
        let d = tempdir().unwrap();
        git_init(d.path());
        fs::write(d.path().join(".gitignore"), "target/").unwrap(); // no trailing newline
        assert!(ensure_gitignored(d.path()).unwrap());
        let gi = fs::read_to_string(d.path().join(".gitignore")).unwrap();
        assert!(gi.starts_with("target/\n"));
        assert!(gi.contains(".commet/"));
    }

    #[test]
    fn ensure_gitignored_skips_without_git() {
        let d = tempdir().unwrap();
        // no .git
        assert!(!ensure_gitignored(d.path()).unwrap());
        assert!(!d.path().join(".gitignore").exists());
    }

    #[test]
    fn clear_removes_live_file_and_archives() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        // live + rotate to .1 by exceeding a tiny threshold.
        append_with_limit(&path, &record("one"), 10).unwrap();
        append_with_limit(&path, &record("two"), 10).unwrap();
        assert!(archive_path(&path, 1).exists());

        assert!(clear(&path).unwrap());
        assert!(!path.exists());
        assert!(!archive_path(&path, 1).exists());
        // Nothing left to clear.
        assert!(!clear(&path).unwrap());
    }

    #[test]
    fn drop_last_removes_only_the_newest_record() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        for i in 1..=3 {
            let mut r = record(&format!("m{i}"));
            r.ts = format!("2026-07-12T00:00:0{i}Z");
            append(&path, &r).unwrap();
        }

        assert!(drop_last(std::slice::from_ref(&path)).unwrap());
        let left = load(&path).unwrap();
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|r| r.edited_text != "m3")); // newest gone
        assert_eq!(left[0].edited_text, "m1");

        // Draining to empty deletes the file; then nothing to drop.
        assert!(drop_last(std::slice::from_ref(&path)).unwrap());
        assert!(drop_last(std::slice::from_ref(&path)).unwrap());
        assert!(!drop_last(&[path]).unwrap());
    }

    #[test]
    fn select_examples_dedups_newest_first_and_filters_format() {
        let mut recs = Vec::new();
        for (fmt, text) in [
            ("conventional", "feat: a"), // oldest
            ("gitmoji", "✨ b"),
            ("conventional", "feat: a"), // duplicate, newer
            ("conventional", "fix: c"),  // newest conventional
        ] {
            let mut r = record(text);
            r.format = fmt.into();
            recs.push(r);
        }

        assert_eq!(
            select_examples(&recs, "conventional", 5),
            vec!["fix: c".to_string(), "feat: a".to_string()]
        );
        assert_eq!(select_examples(&recs, "conventional", 1), vec!["fix: c"]);
        assert!(select_examples(&recs, "plain", 5).is_empty());
    }

    #[test]
    fn repo_path_is_under_dot_commet() {
        let p = repo_store_path(Path::new("/work/repo"));
        assert_eq!(p, PathBuf::from("/work/repo/.commet/history.jsonl"));
    }

    // ---------- Store (scope filter) ----------

    /// Seed a repo + global file with a distinguishable record each and
    /// return their paths.
    fn seeded_layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("repo").join("history.jsonl");
        let global = dir.path().join("global").join("history.jsonl");
        append(&repo, &record("repo entry")).unwrap();
        append(&global, &record("global entry")).unwrap();
        (dir, repo, global)
    }

    #[test]
    fn read_scope_off_reads_nothing() {
        let (_d, repo, global) = seeded_layout();
        let store = Store::with_paths(LearningScope::Off, Some(repo), Some(global));
        assert!(store.read().unwrap().is_empty());
        assert!(!store.is_enabled());
    }

    #[test]
    fn read_scope_repo_reads_only_repo() {
        let (_d, repo, global) = seeded_layout();
        let store = Store::with_paths(LearningScope::Repo, Some(repo), Some(global));
        let got = store.read().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].edited_text, "repo entry");
        assert!(store.is_enabled());
    }

    #[test]
    fn read_scope_global_reads_only_global() {
        let (_d, repo, global) = seeded_layout();
        let store = Store::with_paths(LearningScope::Global, Some(repo), Some(global));
        let got = store.read().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].edited_text, "global entry");
    }

    #[test]
    fn read_scope_repo_global_reads_both_repo_ranked_first() {
        let (_d, repo, global) = seeded_layout();
        let store = Store::with_paths(LearningScope::RepoGlobal, Some(repo), Some(global));
        let got = store.read().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].edited_text, "repo entry");
        assert_eq!(got[1].edited_text, "global entry");
    }

    /// Fresh (unwritten) repo + global paths.
    fn empty_layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("repo").join("history.jsonl");
        let global = dir.path().join("global").join("history.jsonl");
        (dir, repo, global)
    }

    #[test]
    fn write_scope_off_writes_nothing() {
        let (_d, repo, global) = empty_layout();
        let store = Store::with_paths(LearningScope::Off, Some(repo.clone()), Some(global.clone()));
        store.write(&record("x")).unwrap();
        assert!(!repo.exists());
        assert!(!global.exists());
    }

    #[test]
    fn write_scope_repo_writes_only_repo() {
        let (_d, repo, global) = empty_layout();
        let store = Store::with_paths(
            LearningScope::Repo,
            Some(repo.clone()),
            Some(global.clone()),
        );
        store.write(&record("x")).unwrap();
        assert!(repo.exists());
        assert!(!global.exists());
    }

    #[test]
    fn write_scope_global_writes_only_global() {
        let (_d, repo, global) = empty_layout();
        let store = Store::with_paths(
            LearningScope::Global,
            Some(repo.clone()),
            Some(global.clone()),
        );
        store.write(&record("x")).unwrap();
        assert!(!repo.exists());
        assert!(global.exists());
    }

    #[test]
    fn write_scope_repo_global_writes_both() {
        let (_d, repo, global) = empty_layout();
        let store = Store::with_paths(
            LearningScope::RepoGlobal,
            Some(repo.clone()),
            Some(global.clone()),
        );
        store.write(&record("x")).unwrap();
        assert!(repo.exists());
        assert!(global.exists());
    }

    #[test]
    fn repo_scope_outside_a_repo_is_a_noop() {
        // `repo` scope with no repo path resolved: nothing to read/write.
        let store = Store::with_paths(LearningScope::Repo, None, global_store_path());
        assert!(store.read().unwrap().is_empty());
        store.write(&record("x")).unwrap(); // must not panic or error
    }
}
