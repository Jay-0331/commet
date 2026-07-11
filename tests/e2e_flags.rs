//! End-to-end smoke tests for the default generate flow, one per v0.1
//! flag, driven through the offline mock provider.
//!
//! Each test builds a throwaway git repo, stages a change, sets
//! `COMMITCRAFTER_MOCK_RESPONSE` (so `provider::registry` returns the
//! mock), runs `cc`, and asserts the observable: stdout, the created
//! commit, or the prompt recorded to `COMMITCRAFTER_MOCK_LOG`.
//!
//! Compiled only with the `mock` feature (CI runs `--all-features`);
//! without it the whole file is empty.
#![cfg(feature = "mock")]

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::Command as AssertCommand;
use tempfile::TempDir;

/// A throwaway git repo with committer identity and signing disabled.
fn repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q"]);
    git(p, &["config", "user.email", "t@example.com"]);
    git(p, &["config", "user.name", "Tester"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    dir
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

/// Write and stage a file.
fn stage(dir: &Path, name: &str, contents: &str) {
    fs::write(dir.join(name), contents).unwrap();
    git(dir, &["add", name]);
}

/// A `cc` command rooted in `dir` with the mock response set.
fn cc(dir: &Path, response: &str) -> AssertCommand {
    let mut cmd = AssertCommand::cargo_bin("cc").unwrap();
    cmd.current_dir(dir);
    cmd.env("COMMITCRAFTER_MOCK_RESPONSE", response);
    cmd
}

/// Subject line of HEAD, or `None` when there are no commits yet.
fn head_subject(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["log", "-1", "--pretty=%s"])
        .output()
        .unwrap();
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// Read the JSON request the mock recorded at `log`.
fn logged_request(log: &Path) -> serde_json::Value {
    let raw = fs::read_to_string(log).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn print_outputs_message_without_committing() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");

    cc(dir.path(), "feat: add a.txt")
        .arg("--print")
        .assert()
        .success()
        .stdout(predicates::str::contains("feat: add a.txt"));

    assert_eq!(head_subject(dir.path()), None, "--print must not commit");
}

#[test]
fn yes_commits_the_message() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");

    cc(dir.path(), "feat: add greeting")
        .arg("-y")
        .assert()
        .success();

    assert_eq!(
        head_subject(dir.path()).as_deref(),
        Some("feat: add greeting")
    );
}

#[test]
fn yes_with_g2_commits_the_first_candidate() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");

    cc(dir.path(), "feat: first\nfeat: second")
        .args(["-y", "-g", "2"])
        .assert()
        .success();

    // `-y` accepts the first candidate.
    assert_eq!(head_subject(dir.path()).as_deref(), Some("feat: first"));
}

#[test]
fn g3_print_shows_three_candidates() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");

    cc(dir.path(), "one\ntwo\nthree")
        .args(["-g", "3", "--print"])
        .assert()
        .success()
        .stdout(predicates::str::contains("candidate 1"))
        .stdout(predicates::str::contains("candidate 2"))
        .stdout(predicates::str::contains("candidate 3"));
}

#[test]
fn type_gitmoji_puts_the_rule_in_the_prompt() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");
    let log = dir.path().join("req.json");

    cc(dir.path(), "✨ add greeting")
        .args(["-t", "gitmoji", "--print"])
        .env("COMMITCRAFTER_MOCK_LOG", &log)
        .assert()
        .success();

    let req = logged_request(&log);
    assert!(
        req["system_prompt"].as_str().unwrap().contains("gitmoji"),
        "system prompt should carry the gitmoji rule"
    );
}

#[test]
fn prompt_flag_appends_user_override() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");
    let log = dir.path().join("req.json");

    cc(dir.path(), "feat: saludo")
        .args(["-p", "write in Spanish", "--print"])
        .env("COMMITCRAFTER_MOCK_LOG", &log)
        .assert()
        .success();

    let system = logged_request(&log)["system_prompt"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(system.contains("USER OVERRIDE"));
    assert!(system.contains("write in Spanish"));
}

#[test]
fn exclude_drops_matching_paths_from_the_diff() {
    let dir = repo();
    stage(dir.path(), "keep.txt", "keep me\n");
    stage(dir.path(), "secret.env", "TOKEN=drop me\n");
    let log = dir.path().join("req.json");

    cc(dir.path(), "chore: update")
        .args(["-x", "*.env", "--print"])
        .env("COMMITCRAFTER_MOCK_LOG", &log)
        .assert()
        .success();

    let user = logged_request(&log)["user_prompt"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(user.contains("keep.txt"), "kept file should be in the diff");
    assert!(
        !user.contains("TOKEN=drop me"),
        "excluded file's contents must not reach the prompt"
    );
}

#[cfg(unix)]
#[test]
fn no_verify_bypasses_a_failing_pre_commit_hook() {
    use std::os::unix::fs::PermissionsExt;

    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");

    // A pre-commit hook that always fails.
    let hook = dir.path().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

    // Without --no-verify the hook blocks the commit.
    cc(dir.path(), "feat: blocked").arg("-y").assert().failure();
    assert_eq!(head_subject(dir.path()), None);

    // With --no-verify the hook is skipped and the commit lands.
    cc(dir.path(), "feat: forced")
        .args(["-y", "-n"])
        .assert()
        .success();
    assert_eq!(head_subject(dir.path()).as_deref(), Some("feat: forced"));
}

#[test]
#[ignore = "clipboard (-c) is not implemented yet — see #55 (arboard)"]
fn clipboard_copies_without_committing() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");
    cc(dir.path(), "feat: copied").arg("-c").assert().success();
    assert_eq!(head_subject(dir.path()), None);
}
