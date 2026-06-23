//! `std::process::Command` wrappers for each `git` operation we use.
//!
//! Every public function takes the working directory explicitly so
//! tests can drive against tempdir repos without touching `cwd`.
//! Errors are uniformly [`Error::Git`] and carry the full argv and
//! stderr so users can copy-paste the offending command into their
//! shell.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

/// Locate the repository root containing `cwd` via
/// `git rev-parse --show-toplevel`. Returns [`Error::Git`] when not
/// inside a repo.
pub fn repo_root(cwd: &Path) -> Result<PathBuf> {
    let stdout = run(
        cwd,
        &[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
    )?;
    let trimmed = stdout.trim_end_matches(['\n', '\r']);
    if trimmed.is_empty() {
        return Err(Error::Git("git rev-parse returned empty output".into()));
    }
    Ok(PathBuf::from(trimmed))
}

/// Run `git diff --cached` and return the unified diff text of
/// everything currently staged in `cwd`. The output may be empty if
/// nothing is staged.
pub fn diff_staged(cwd: &Path) -> Result<String> {
    run(cwd, &[OsStr::new("diff"), OsStr::new("--cached")])
}

/// Stage one or more paths via `git add -- <paths…>`.
///
/// Uses the `--` separator so a path that happens to look like a
/// flag is never reinterpreted.
pub fn add(cwd: &Path, paths: &[&Path]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args: Vec<OsString> = vec![OsString::from("add"), OsString::from("--")];
    args.extend(paths.iter().map(|p| p.as_os_str().to_owned()));
    let arg_refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
    run(cwd, &arg_refs).map(|_| ())
}

/// Unstage one or more paths via `git restore --staged -- <paths…>`.
///
/// Used by the TUI's auto-unstage-on-abort path so when the user
/// quits without committing, the index is returned to whatever it
/// was before `cc` started.
pub fn restore_staged(cwd: &Path, paths: &[&Path]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args: Vec<OsString> = vec![
        OsString::from("restore"),
        OsString::from("--staged"),
        OsString::from("--"),
    ];
    args.extend(paths.iter().map(|p| p.as_os_str().to_owned()));
    let arg_refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
    run(cwd, &arg_refs).map(|_| ())
}

/// Commit using a message file: `git commit -F <path> [--no-verify]`.
///
/// Writing the message to a tempfile (instead of `-m "…"`) keeps the
/// user's `commit.template`, `commit.cleanup`, signing, and hooks
/// working exactly as they would for a normal `git commit`.
pub fn commit(cwd: &Path, message_file: &Path, no_verify: bool) -> Result<()> {
    let mut args: Vec<OsString> = vec![OsString::from("commit"), OsString::from("-F")];
    args.push(message_file.as_os_str().to_owned());
    if no_verify {
        args.push(OsString::from("--no-verify"));
    }
    let arg_refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
    run(cwd, &arg_refs).map(|_| ())
}

/// Run a git command with the given argv inside `cwd`, returning
/// stdout as a `String` on success or [`Error::Git`] with argv +
/// stderr on failure.
pub(crate) fn run(cwd: &Path, args: &[&OsStr]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| Error::Git(format!("spawn git: {e}")))?;

    if !output.status.success() {
        let argv = render_argv(args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim_end();
        return Err(Error::Git(format!(
            "`git {argv}` exited with {}: {stderr}",
            output.status,
        )));
    }

    String::from_utf8(output.stdout).map_err(|e| Error::Git(format!("git stdout not utf-8: {e}")))
}

/// Same as [`run`] but returns the raw bytes of stdout — needed by
/// the porcelain parser, which has to handle NUL separators and
/// non-UTF-8 paths (rare but possible).
pub(crate) fn run_bytes(cwd: &Path, args: &[&OsStr]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| Error::Git(format!("spawn git: {e}")))?;

    if !output.status.success() {
        let argv = render_argv(args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim_end();
        return Err(Error::Git(format!(
            "`git {argv}` exited with {}: {stderr}",
            output.status,
        )));
    }
    Ok(output.stdout)
}

fn render_argv(args: &[&OsStr]) -> String {
    args.iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_argv_joins_with_spaces() {
        let args: Vec<&OsStr> = vec![
            OsStr::new("status"),
            OsStr::new("--porcelain=v1"),
            OsStr::new("-z"),
        ];
        assert_eq!(render_argv(&args), "status --porcelain=v1 -z");
    }

    #[test]
    fn repo_root_outside_a_repo_errors_with_argv_and_stderr() {
        let tmp = tempfile::tempdir().unwrap();
        let err = repo_root(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("git rev-parse --show-toplevel"),
            "error should mention argv; got: {msg}",
        );
    }

    #[test]
    fn add_with_empty_paths_is_a_noop() {
        // Should not even invoke git — succeeds against a non-repo dir.
        let tmp = tempfile::tempdir().unwrap();
        add(tmp.path(), &[]).expect("empty add is a noop");
    }

    #[test]
    fn restore_staged_with_empty_paths_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        restore_staged(tmp.path(), &[]).expect("empty restore_staged is a noop");
    }
}
