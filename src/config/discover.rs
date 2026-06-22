//! Locate the global and per-repo config files.
//!
//! Discovery is deliberately separated from loading so tests can
//! supply explicit paths without touching process-wide environment
//! variables or invoking `git`.
//!
//! The public functions ([`global_config_path`] and
//! [`repo_config_path`]) consult the real environment / shell; the
//! `*_with` helpers underneath them take the relevant inputs as
//! arguments so they are pure functions ideal for unit tests.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Suffix appended to the chosen config directory.
const CONFIG_FILE: &str = "commitcrafter/config.toml";

/// Per-repo config file name, looked up relative to the repo root
/// returned by `git rev-parse --show-toplevel`.
pub const REPO_CONFIG_FILE: &str = ".commitcrafter.toml";

/// Resolve the global config file path, following the XDG Base
/// Directory spec on Linux/macOS:
///
/// 1. `$XDG_CONFIG_HOME/commitcrafter/config.toml` if `XDG_CONFIG_HOME`
///    is set and non-empty.
/// 2. `$HOME/.config/commitcrafter/config.toml` otherwise.
///
/// Returns `None` only if neither variable is set (extremely unusual,
/// but we surface it as "no global config" rather than panicking).
pub fn global_config_path() -> Option<PathBuf> {
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    global_config_path_with(xdg.as_deref(), home.as_deref())
}

/// Pure-function variant of [`global_config_path`] used by tests.
pub fn global_config_path_with(
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(xdg) = xdg_config_home
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join(CONFIG_FILE));
    }
    let home = home?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".config").join(CONFIG_FILE))
}

/// Resolve the per-repo config file path for the current working
/// directory. Returns `None` when not inside a git repository or when
/// `git` is not on `PATH`.
pub fn repo_config_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    repo_config_path_in(&cwd)
}

/// Pure-function variant of [`repo_config_path`] used by tests.
///
/// Runs `git rev-parse --show-toplevel` with `cwd` as the working
/// directory and joins [`REPO_CONFIG_FILE`] onto the result. Returns
/// `None` for any non-success exit code.
pub fn repo_config_path_in(cwd: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = std::str::from_utf8(&out.stdout).ok()?.trim();
    if root.is_empty() {
        return None;
    }
    Some(PathBuf::from(root).join(REPO_CONFIG_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_takes_precedence_over_home() {
        let p = global_config_path_with(Some("/x/cfg"), Some("/h")).unwrap();
        assert_eq!(p, PathBuf::from("/x/cfg/commitcrafter/config.toml"));
    }

    #[test]
    fn empty_xdg_falls_back_to_home() {
        let p = global_config_path_with(Some(""), Some("/h")).unwrap();
        assert_eq!(p, PathBuf::from("/h/.config/commitcrafter/config.toml"));
    }

    #[test]
    fn no_xdg_no_home_yields_none() {
        assert!(global_config_path_with(None, None).is_none());
        assert!(global_config_path_with(Some(""), None).is_none());
        assert!(global_config_path_with(Some(""), Some("")).is_none());
    }

    #[test]
    fn missing_xdg_uses_home() {
        let p = global_config_path_with(None, Some("/users/alice")).unwrap();
        assert_eq!(
            p,
            PathBuf::from("/users/alice/.config/commitcrafter/config.toml"),
        );
    }

    #[test]
    fn repo_path_in_a_real_repo() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let init = Command::new("git")
            .current_dir(tmp.path())
            .args(["init", "--quiet"])
            .status()
            .expect("git init runs");
        assert!(init.success(), "git init failed in {:?}", tmp.path());

        let resolved = repo_config_path_in(tmp.path()).expect("repo path resolved");
        assert_eq!(resolved.file_name().unwrap(), REPO_CONFIG_FILE);

        // Canonicalize the *parent* directory only — the config file
        // itself doesn't exist on disk yet.
        let resolved_parent = resolved.parent().expect("has parent");
        let expected_root = tmp.path().canonicalize().expect("canonicalize tempdir");
        let actual_root = resolved_parent
            .canonicalize()
            .expect("canonicalize repo root");
        assert_eq!(actual_root, expected_root);
    }

    #[test]
    fn repo_path_outside_a_repo_returns_none() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        assert!(repo_config_path_in(tmp.path()).is_none());
    }
}
