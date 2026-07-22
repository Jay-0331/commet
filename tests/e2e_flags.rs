//! End-to-end smoke tests for the default generate flow, one per v0.1
//! flag, driven through the offline mock provider.
//!
//! Each test builds a throwaway git repo, stages a change, sets
//! `COMMET_MOCK_RESPONSE` (so `provider::registry` returns the
//! mock), runs `cc`, and asserts the observable: stdout, the created
//! commit, or the prompt recorded to `COMMET_MOCK_LOG`.
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

/// A `cc` command rooted in `dir` with the mock response set. Learning
/// is turned off by default so a stray `-y` in these tests never writes
/// to the developer's real global history store; the recording test
/// re-enables it with a repo scope confined to its tempdir.
fn cc(dir: &Path, response: &str) -> AssertCommand {
    let mut cmd = AssertCommand::cargo_bin("commet").unwrap();
    cmd.current_dir(dir);
    cmd.env("COMMET_MOCK_RESPONSE", response);
    cmd.args(["--set", "learning.scope=off"]);
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

fn staged_names(dir: &Path) -> Vec<String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(["diff", "--cached", "--name-only"])
        .output()
        .unwrap();
    assert!(output.status.success(), "git diff --cached failed");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
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
fn generate_flag_reaches_provider_and_prints_three_candidates() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");
    let log = dir.path().join("req.json");

    cc(dir.path(), "one\ntwo\nthree")
        .args(["-g", "3", "--print"])
        .env("COMMET_MOCK_LOG", &log)
        .assert()
        .success()
        .stdout(predicates::str::contains("candidate 1"))
        .stdout(predicates::str::contains("candidate 2"))
        .stdout(predicates::str::contains("candidate 3"));

    assert_eq!(logged_request(&log)["n"], 3);
}

#[test]
fn configured_generate_count_is_used_without_the_flag() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");
    let log = dir.path().join("req.json");

    cc(dir.path(), "first\nsecond")
        .args(["--set", "style.generate=2", "--print"])
        .env("COMMET_MOCK_LOG", &log)
        .assert()
        .success()
        .stdout(predicates::str::contains("candidate 1"))
        .stdout(predicates::str::contains("candidate 2"));

    assert_eq!(logged_request(&log)["n"], 2);
}

#[test]
fn type_gitmoji_puts_the_rule_in_the_prompt() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");
    let log = dir.path().join("req.json");

    cc(dir.path(), "✨ add greeting")
        .args(["-t", "gitmoji", "--print"])
        .env("COMMET_MOCK_LOG", &log)
        .assert()
        .success();

    let req = logged_request(&log);
    assert!(
        req["system_prompt"].as_str().unwrap().contains("gitmoji"),
        "system prompt should carry the gitmoji rule"
    );
}

#[test]
fn type_custom_uses_configured_prompt_and_template_for_one_run() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");
    let config = r#"
[style]
format = "plain"

[style.custom]
system_prompt = "CUSTOM RELEASE RULES"
template = "<type>: <summary>\n\n<body>"
"#;
    fs::write(dir.path().join(".commet.toml"), config).unwrap();

    let custom_log = dir.path().join("custom.json");
    cc(dir.path(), "release: custom")
        .args(["-t", "custom", "--print"])
        .env("COMMET_MOCK_LOG", &custom_log)
        .assert()
        .success();

    assert_eq!(
        logged_request(&custom_log)["system_prompt"],
        "CUSTOM RELEASE RULES\n\nOutput template (follow exactly):\n<type>: <summary>\n\n<body>"
    );

    let plain_log = dir.path().join("plain.json");
    cc(dir.path(), "plain summary")
        .arg("--print")
        .env("COMMET_MOCK_LOG", &plain_log)
        .assert()
        .success();

    let plain_request = logged_request(&plain_log);
    let plain = plain_request["system_prompt"].as_str().unwrap();
    assert!(plain.contains("single concise sentence"));
    assert!(!plain.contains("CUSTOM RELEASE RULES"));
    assert_eq!(
        fs::read_to_string(dir.path().join(".commet.toml")).unwrap(),
        config
    );
}

#[test]
fn prompt_flag_appends_user_override() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");
    let log = dir.path().join("req.json");

    cc(dir.path(), "feat: saludo")
        .args(["-p", "write in Spanish", "--print"])
        .env("COMMET_MOCK_LOG", &log)
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
fn exclude_merges_cli_and_config_filters_without_unstaging() {
    let dir = repo();
    stage(dir.path(), "keep.txt", "keep me\n");
    stage(dir.path(), "secret.env", "TOKEN=drop me\n");
    stage(dir.path(), "generated.rs", "generated cli exclusion\n");
    stage(dir.path(), "Cargo.lock", "configured exclusion\n");
    let log = dir.path().join("req.json");

    cc(dir.path(), "chore: update")
        .args(["-x", "*.env", "-x", "generated.*", "--print"])
        .env("COMMET_MOCK_LOG", &log)
        .assert()
        .success();

    let user = logged_request(&log)["user_prompt"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        user.contains("keep.txt"),
        "kept path should reach the prompt"
    );
    assert!(
        user.contains("keep me"),
        "kept diff should reach the prompt"
    );
    for excluded in [
        "secret.env",
        "TOKEN=drop me",
        "generated.rs",
        "generated cli exclusion",
        "Cargo.lock",
        "configured exclusion",
    ] {
        assert!(
            !user.contains(excluded),
            "excluded prompt data leaked: {excluded}\n{user}"
        );
    }

    assert_eq!(
        staged_names(dir.path()),
        ["Cargo.lock", "generated.rs", "keep.txt", "secret.env"],
        "exclude must not change the git index"
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
fn yes_records_the_accepted_commit_to_the_repo_store() {
    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");

    // Override the helper's `scope=off` with a repo scope — the store
    // then lives inside this tempdir, never the real global path.
    cc(dir.path(), "feat: recorded")
        .args(["-y", "--set", "learning.scope=repo"])
        .assert()
        .success();

    let store = dir.path().join(".commet/history.jsonl");
    let content = fs::read_to_string(&store).expect("history file written");
    let rec: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();

    assert_eq!(rec["edited_text"], "feat: recorded");
    assert_eq!(rec["accepted_index"], 0);
    assert!(
        rec["ts"].as_str().unwrap().ends_with('Z'),
        "ts is ISO-8601 UTC"
    );
    assert!(
        rec["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "a.txt")
    );
}

struct ClipboardRestore {
    previous: Option<String>,
}

impl ClipboardRestore {
    fn capture() -> Option<Self> {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        Some(Self {
            previous: clipboard.get_text().ok(),
        })
    }
}

impl Drop for ClipboardRestore {
    fn drop(&mut self) {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return;
        };
        if let Some(previous) = self.previous.take() {
            let _ = clipboard.set_text(previous);
        } else {
            let _ = clipboard.clear();
        }
    }
}

#[test]
fn clipboard_with_multiple_candidates_copies_first_headlessly_without_committing() {
    let Some(_restore) = ClipboardRestore::capture() else {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must provide a clipboard display"
        );
        eprintln!("skipping clipboard assertion: no display is available");
        return;
    };

    let dir = repo();
    stage(dir.path(), "a.txt", "hello\n");
    cc(dir.path(), "feat: copied\nfix: second\ndocs: third")
        .args(["-c", "-g", "3"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Copied: feat: copied"));

    let mut clipboard = arboard::Clipboard::new().expect("clipboard remains available");
    assert_eq!(clipboard.get_text().unwrap(), "feat: copied");
    assert_eq!(head_subject(dir.path()), None);
}
