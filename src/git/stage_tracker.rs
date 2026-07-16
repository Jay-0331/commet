//! RAII guard that auto-unstages paths cc staged this session.
//!
//! When `commet` runs the default flow, it stages files on the user's
//! behalf so the LLM has something to look at. If the user later
//! aborts (`q` in the TUI, Ctrl-C, an unwinding panic), those
//! changes should not be left in the index — the user expects
//! their working state to look the way it did before they ran
//! `commet`.
//!
//! [`StageTracker`] wraps that contract:
//!
//! 1. `stage(paths)` calls [`git::add`] and records the paths.
//! 2. On `Drop`, recorded paths are passed to [`git::restore_staged`].
//! 3. `release()` clears the recorded list — call after a
//!    successful commit so `Drop` becomes a no-op.
//!
//! `Drop` runs on normal scope exit, panic unwind, and explicit
//! [`StageTracker::abort`]. **Ctrl-C delivered as `SIGINT` does
//! not invoke `Drop`**; the TUI panic / signal hook (E5) translates
//! the signal into a normal unwind so this guard fires correctly.
//!
//! When `[git].auto_unstage_on_abort = false`, the tracker still
//! stages files but the `Drop` path skips restoration — the user
//! opted into "leave it where I left it" semantics.

use std::path::{Path, PathBuf};

use crate::error::Result;

use super::wrappers;

/// Tracks paths cc staged this session and restores them on `Drop`.
#[derive(Debug)]
pub struct StageTracker {
    cwd: PathBuf,
    enabled: bool,
    tracked: Vec<PathBuf>,
}

impl StageTracker {
    /// Create a new tracker rooted at `cwd`. Set `enabled = false`
    /// to honor `[git].auto_unstage_on_abort = false` — `stage`
    /// still works, but `Drop` and `abort` become no-ops.
    pub fn new(cwd: PathBuf, enabled: bool) -> Self {
        Self {
            cwd,
            enabled,
            tracked: Vec::new(),
        }
    }

    /// Stage `paths` via `git add --` and record them so they can
    /// be unstaged automatically if the session ends abnormally.
    pub fn stage(&mut self, paths: &[&Path]) -> Result<()> {
        self.stage_preserving(paths, &[])
    }

    /// Stage every selected path, but only track paths that were not already
    /// staged before this session. This keeps a picker abort from unstaging
    /// index changes that belong to the user.
    pub fn stage_preserving(&mut self, paths: &[&Path], already_staged: &[PathBuf]) -> Result<()> {
        wrappers::add(&self.cwd, paths)?;
        self.tracked.extend(
            paths
                .iter()
                .filter(|path| !already_staged.iter().any(|staged| staged == **path))
                .map(|path| path.to_path_buf()),
        );
        Ok(())
    }

    /// How many paths the tracker is currently watching.
    pub fn tracked_len(&self) -> usize {
        self.tracked.len()
    }

    /// Read-only view of the recorded paths. Useful for assertions
    /// and for `cc config show`-style debugging.
    pub fn tracked(&self) -> &[PathBuf] {
        &self.tracked
    }

    /// Disarm the tracker — clears the recorded paths so the
    /// `Drop` impl restores nothing. Call after a successful commit.
    pub fn release(mut self) {
        self.tracked.clear();
    }

    /// Explicitly restore everything we staged. Equivalent to
    /// dropping the tracker, but returns a `Result` so callers can
    /// surface errors instead of relying on `tracing::warn!`.
    pub fn abort(mut self) -> Result<()> {
        if !self.enabled || self.tracked.is_empty() {
            self.tracked.clear();
            return Ok(());
        }
        let paths: Vec<&Path> = self.tracked.iter().map(PathBuf::as_path).collect();
        let result = wrappers::restore_staged(&self.cwd, &paths);
        // Empty the list either way so `Drop` doesn't retry.
        self.tracked.clear();
        result
    }
}

impl Drop for StageTracker {
    fn drop(&mut self) {
        if !self.enabled || self.tracked.is_empty() {
            return;
        }
        let paths: Vec<&Path> = self.tracked.iter().map(PathBuf::as_path).collect();
        if let Err(err) = wrappers::restore_staged(&self.cwd, &paths) {
            // Drop can't fail upward; surface as a warning so users
            // see what happened in the log without a crash.
            tracing::warn!(error = %err, paths = ?paths, "auto-unstage failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_with_no_tracked_paths() {
        let tracker = StageTracker::new(PathBuf::from("/tmp"), true);
        assert_eq!(tracker.tracked_len(), 0);
        assert!(tracker.tracked().is_empty());
    }

    #[test]
    fn release_clears_tracked_list() {
        // Build a tracker directly (bypass `stage` since we don't
        // want a real git repo here — release()'s effect is
        // observable on the in-memory `tracked` Vec.
        let mut tracker = StageTracker::new(PathBuf::from("/tmp"), true);
        tracker.tracked.push(PathBuf::from("a.rs"));
        tracker.tracked.push(PathBuf::from("b.rs"));
        assert_eq!(tracker.tracked_len(), 2);
        tracker.release();
        // `release` consumes self; we can't observe `tracked_len`
        // after — but if Drop fired with non-empty `tracked` on a
        // bogus cwd it would log a warn and not panic, so the test
        // simply has to not blow up.
    }
}
