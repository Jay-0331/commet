use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn failing_check_prints_json_and_exits_five() {
    let dir = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir.path())
        .status()
        .unwrap();

    let mut cmd = Command::cargo_bin("commet").unwrap();
    cmd.current_dir(dir.path())
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_STATE_HOME", dir.path().join("state"))
        .env_remove("ANTHROPIC_API_KEY")
        .arg("doctor")
        .arg("--json")
        .assert()
        .code(5)
        .stdout(predicate::str::contains("\"name\": \"provider API key\""))
        .stdout(predicate::str::contains("\"status\": \"fail\""));
}

#[test]
fn full_without_key_reports_skipped_smoke_check() {
    let dir = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir.path())
        .status()
        .unwrap();

    let mut cmd = Command::cargo_bin("commet").unwrap();
    cmd.current_dir(dir.path())
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_STATE_HOME", dir.path().join("state"))
        .env_remove("ANTHROPIC_API_KEY")
        .args(["doctor", "--full", "--json"])
        .assert()
        .code(5)
        .stdout(predicate::str::contains(
            "\"name\": \"provider smoke completion\"",
        ))
        .stdout(predicate::str::contains("\"status\": \"warn\""))
        .stdout(predicate::str::contains(
            "skipped: the provider API key is missing",
        ));
}
