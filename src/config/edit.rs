//! `cc config edit` — pick the right config file and open it in
//! `$EDITOR`, scaffolding a commented starter template if it doesn't
//! exist yet.
//!
//! Target selection (low → high priority):
//!
//! 1. **Auto** (default) — per-repo `.commet.toml` when inside
//!    a git repo; otherwise the global file under
//!    `$XDG_CONFIG_HOME/commet/config.toml`.
//! 2. **`--repo`** — force the per-repo file; error if not inside a
//!    repo.
//! 3. **`--global`** — force the global file; error if `$HOME` /
//!    `$XDG_CONFIG_HOME` are both unavailable.
//!
//! Pure target-picking lives in [`pick_target`] so it can be unit
//! tested without touching the environment or running `git`. The
//! [`run`] wrapper plugs in the real discovery helpers and the
//! editor spawn.

use std::path::{Path, PathBuf};

use crate::cli::ConfigEditArgs;
use crate::editor;
use crate::error::{Error, Result};

use super::discover;
use super::schema::Config;

/// Banner inserted at the top of a newly-scaffolded config file.
pub const TEMPLATE_HEADER: &str = "\
# commet configuration
#
# `cc config show` prints the merged effective values from this file
# plus the global config (~/.config/commet/config.toml).
#
# API keys are NEVER stored here. Set them via environment variables:
#   ANTHROPIC_API_KEY   OPENAI_API_KEY   OPENROUTER_API_KEY
#

";

/// Dispatch entry point. Resolves the target via [`pick_target`],
/// scaffolds the file if missing, and spawns the user's `$EDITOR`.
pub fn run(args: &ConfigEditArgs) -> Result<()> {
    let target = pick_target(
        args,
        discover::repo_config_path(),
        discover::global_config_path(),
    )?;

    if !target.exists() {
        scaffold(&target)?;
    }

    editor::spawn(&target)
}

/// Pure target picker. `repo_path` and `global_path` are passed in
/// so tests don't need to manipulate the environment or run `git`.
pub fn pick_target(
    args: &ConfigEditArgs,
    repo_path: Option<PathBuf>,
    global_path: Option<PathBuf>,
) -> Result<PathBuf> {
    if args.global {
        return global_path.ok_or_else(no_global);
    }
    if args.repo {
        return repo_path.ok_or_else(no_repo);
    }
    // Auto: prefer the per-repo file when inside a repo.
    if let Some(p) = repo_path {
        return Ok(p);
    }
    global_path.ok_or_else(no_global)
}

/// Write a starter config to `path`, creating parent directories as
/// needed. Does not overwrite an existing file.
pub fn scaffold(path: &Path) -> Result<()> {
    if path.exists() {
        return Err(Error::Config(format!(
            "refusing to scaffold over existing file {}",
            path.display(),
        )));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Config(format!("create {}: {e}", parent.display())))?;
    }
    let body = format!("{TEMPLATE_HEADER}{}", Config::default().to_toml_string()?,);
    std::fs::write(path, body)
        .map_err(|e| Error::Config(format!("write {}: {e}", path.display())))?;
    Ok(())
}

fn no_global() -> Error {
    Error::Config(
        "no global config path available (\
         set $XDG_CONFIG_HOME or $HOME to choose where the file lives)"
            .into(),
    )
}

fn no_repo() -> Error {
    Error::Config(
        "not inside a git repository (\
         `cc config edit --repo` requires a repo; run from inside one or drop the flag)"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(global: bool, repo: bool) -> ConfigEditArgs {
        ConfigEditArgs { global, repo }
    }

    #[test]
    fn auto_prefers_repo_when_available() {
        let target = pick_target(
            &args(false, false),
            Some(PathBuf::from("/repo/.commet.toml")),
            Some(PathBuf::from("/home/user/.config/commet/config.toml")),
        )
        .unwrap();
        assert_eq!(target, PathBuf::from("/repo/.commet.toml"));
    }

    #[test]
    fn auto_falls_back_to_global_when_outside_repo() {
        let global = PathBuf::from("/home/user/.config/commet/config.toml");
        let target = pick_target(&args(false, false), None, Some(global.clone())).unwrap();
        assert_eq!(target, global);
    }

    #[test]
    fn auto_errors_when_neither_available() {
        let err = pick_target(&args(false, false), None, None).unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("no global config path")));
    }

    #[test]
    fn global_flag_forces_global_even_inside_repo() {
        let global = PathBuf::from("/home/user/.config/commet/config.toml");
        let target = pick_target(
            &args(true, false),
            Some(PathBuf::from("/repo/.commet.toml")),
            Some(global.clone()),
        )
        .unwrap();
        assert_eq!(target, global);
    }

    #[test]
    fn global_flag_errors_when_no_global_available() {
        let err =
            pick_target(&args(true, false), Some(PathBuf::from("/anything")), None).unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("no global config path")));
    }

    #[test]
    fn repo_flag_forces_repo() {
        let repo = PathBuf::from("/repo/.commet.toml");
        let target = pick_target(
            &args(false, true),
            Some(repo.clone()),
            Some(PathBuf::from("/anywhere")),
        )
        .unwrap();
        assert_eq!(target, repo);
    }

    #[test]
    fn repo_flag_errors_when_outside_repo() {
        let err =
            pick_target(&args(false, true), None, Some(PathBuf::from("/anywhere"))).unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("not inside a git repository")));
    }

    #[test]
    fn scaffold_creates_file_with_template_and_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested/dir/config.toml");
        scaffold(&path).unwrap();
        assert!(path.exists(), "scaffold should create the file");

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# commet configuration"));
        assert!(
            text.contains("ANTHROPIC_API_KEY"),
            "header should mention env vars",
        );

        // Stripping the header should leave valid TOML that parses to
        // `Config::default`.
        let toml_body = text
            .split_once(TEMPLATE_HEADER)
            .map(|(_, after)| after.to_string())
            .unwrap_or(text.clone());
        let parsed = Config::from_toml_str(&toml_body).unwrap();
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn scaffold_refuses_to_overwrite_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, b"keep me").unwrap();

        let err = scaffold(&path).unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("refusing to scaffold")));

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, "keep me");
    }
}
