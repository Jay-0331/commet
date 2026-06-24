//! End-to-end smoke tests for [`commitcrafter::git`] against real
//! `git` binaries in tempdir repositories.
//!
//! Each test sets author + committer identity via environment
//! variables (per-Command, not process-wide) so commits succeed
//! without depending on the host's `~/.gitconfig`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use commitcrafter::git::{self, FileStatus};

/// Initialize a fresh git repo in a tempdir.
fn make_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().to_path_buf();
    let status = Command::new("git")
        .current_dir(&path)
        .args(["init", "--quiet", "--initial-branch=main"])
        .status()
        .expect("git init runs");
    assert!(status.success(), "git init failed in {path:?}");

    // Local identity so `git commit` doesn't trip over a missing
    // global config.
    for (k, v) in [
        ("user.email", "tests@commitcrafter.invalid"),
        ("user.name", "commitcrafter tests"),
        ("commit.gpgsign", "false"),
        ("init.defaultBranch", "main"),
    ] {
        let s = Command::new("git")
            .current_dir(&path)
            .args(["config", "--local", k, v])
            .status()
            .expect("git config runs");
        assert!(s.success(), "git config {k}={v} failed");
    }

    (tmp, path)
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn make_initial_commit(root: &Path) {
    write(root, "seed.txt", "seed\n");
    git::add(root, &[Path::new("seed.txt")]).expect("add seed");
    let msg_path = root.join(".cc-msg");
    fs::write(&msg_path, "seed\n").unwrap();
    git::commit(root, &msg_path, /*no_verify=*/ false).expect("seed commit");
    fs::remove_file(&msg_path).ok();
}

#[test]
fn repo_root_resolves_to_init_path() {
    let (_tmp, root) = make_repo();
    let resolved = git::repo_root(&root).unwrap();
    assert_eq!(
        resolved.canonicalize().unwrap(),
        root.canonicalize().unwrap(),
    );
}

#[test]
fn repo_root_outside_a_repo_errors_with_argv() {
    let tmp = tempfile::tempdir().unwrap();
    let err = git::repo_root(tmp.path()).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("rev-parse"),
        "error should mention failing argv; got: {msg}",
    );
}

#[test]
fn status_porcelain_picks_up_untracked_and_modified_files() {
    let (_tmp, root) = make_repo();
    make_initial_commit(&root);

    // Modify the seed and add a new untracked file.
    write(&root, "seed.txt", "seed\nmore\n");
    write(&root, "notes.md", "untracked\n");

    let entries = git::status_porcelain(&root).unwrap();
    let by_path: std::collections::HashMap<_, _> = entries
        .iter()
        .map(|e| (e.path.clone(), e.status.clone()))
        .collect();

    assert_eq!(
        by_path.get(&PathBuf::from("seed.txt")),
        Some(&FileStatus::Modified),
    );
    assert_eq!(
        by_path.get(&PathBuf::from("notes.md")),
        Some(&FileStatus::Untracked),
    );
}

#[test]
fn status_porcelain_surfaces_rename_with_both_paths() {
    let (_tmp, root) = make_repo();
    make_initial_commit(&root);

    // Track an additional file so we have something to rename.
    write(&root, "before.rs", "// will be renamed\n");
    git::add(&root, &[Path::new("before.rs")]).unwrap();
    let msg_path = root.join(".cc-msg");
    std::fs::write(&msg_path, "add before.rs\n").unwrap();
    git::commit(&root, &msg_path, /*no_verify=*/ false).unwrap();
    std::fs::remove_file(&msg_path).ok();

    // Now rename it and observe the porcelain output.
    let mv = Command::new("git")
        .current_dir(&root)
        .args(["mv", "before.rs", "after.rs"])
        .status()
        .unwrap();
    assert!(mv.success(), "git mv failed");

    let entries = git::status_porcelain(&root).unwrap();
    let rename = entries
        .iter()
        .find(|e| matches!(e.status, commitcrafter::git::FileStatus::Renamed { .. }))
        .expect("rename entry present in status output");

    match &rename.status {
        commitcrafter::git::FileStatus::Renamed { from, to } => {
            assert_eq!(from, &PathBuf::from("before.rs"));
            assert_eq!(to, &PathBuf::from("after.rs"));
        }
        other => panic!("expected Renamed, got {other:?}"),
    }
    // The primary `path` matches the new location, matching what git
    // prints first in porcelain output.
    assert_eq!(rename.path, PathBuf::from("after.rs"));
}

#[test]
fn add_then_diff_staged_returns_the_change() {
    let (_tmp, root) = make_repo();
    make_initial_commit(&root);

    write(&root, "src/main.rs", "fn main() {}\n");
    git::add(&root, &[Path::new("src/main.rs")]).unwrap();

    let diff = git::diff_staged(&root).unwrap();
    assert!(diff.contains("src/main.rs"), "diff missing path:\n{diff}");
    assert!(diff.contains("fn main()"), "diff missing content:\n{diff}");
}

#[test]
fn restore_staged_unstages_what_add_staged() {
    let (_tmp, root) = make_repo();
    make_initial_commit(&root);

    write(&root, "src/lib.rs", "pub fn foo() {}\n");
    git::add(&root, &[Path::new("src/lib.rs")]).unwrap();

    // Confirm it's staged…
    let staged_before = git::diff_staged(&root).unwrap();
    assert!(staged_before.contains("src/lib.rs"));

    // …then restore and confirm it's gone from the index.
    git::restore_staged(&root, &[Path::new("src/lib.rs")]).unwrap();
    let staged_after = git::diff_staged(&root).unwrap();
    assert!(
        !staged_after.contains("src/lib.rs"),
        "restore_staged didn't unstage: {staged_after}",
    );
}

#[test]
fn commit_creates_a_real_commit_with_the_message() {
    let (_tmp, root) = make_repo();
    make_initial_commit(&root);

    write(&root, "feature.rs", "// new feature\n");
    git::add(&root, &[Path::new("feature.rs")]).unwrap();

    let msg_path = root.join(".cc-test-msg");
    fs::write(&msg_path, "feat: add feature\n\nbody line\n").unwrap();
    git::commit(&root, &msg_path, /*no_verify=*/ false).unwrap();
    fs::remove_file(&msg_path).ok();

    // Confirm the commit is reachable from HEAD with our message.
    let out = Command::new("git")
        .current_dir(&root)
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .unwrap();
    assert!(out.status.success(), "git log failed");
    let msg = String::from_utf8(out.stdout).unwrap();
    assert!(msg.starts_with("feat: add feature"), "got: {msg:?}");
    assert!(msg.contains("body line"), "body missing: {msg:?}");
}

#[test]
fn commit_no_verify_passes_through_flag() {
    // Drop a pre-commit hook that always fails; the flag should
    // bypass it.
    let (_tmp, root) = make_repo();
    make_initial_commit(&root);

    let hook_dir = root.join(".git").join("hooks");
    let hook_path = hook_dir.join("pre-commit");
    fs::write(&hook_path, "#!/bin/sh\nexit 1\n").unwrap();
    // Make executable (Unix only; CI runners are Unix).
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(&hook_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&hook_path, perms).unwrap();

    write(&root, "skip.rs", "// skip hook\n");
    git::add(&root, &[Path::new("skip.rs")]).unwrap();

    let msg_path = root.join(".cc-test-msg");
    fs::write(&msg_path, "chore: bypass hook\n").unwrap();

    // Without --no-verify the hook should make the commit fail.
    let err = git::commit(&root, &msg_path, /*no_verify=*/ false);
    assert!(err.is_err(), "expected pre-commit hook to fail the commit");

    // With --no-verify it should succeed.
    git::commit(&root, &msg_path, /*no_verify=*/ true).expect("no-verify should bypass");
    fs::remove_file(&msg_path).ok();
}

#[test]
fn add_with_invalid_path_surfaces_argv_and_stderr() {
    let (_tmp, root) = make_repo();
    let err = git::add(&root, &[Path::new("does/not/exist.rs")]).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("add"), "argv missing: {msg}");
}

#[test]
fn diff_staged_of_clean_repo_is_empty() {
    let (_tmp, root) = make_repo();
    make_initial_commit(&root);
    assert!(git::diff_staged(&root).unwrap().is_empty());
}
