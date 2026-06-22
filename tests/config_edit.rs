//! End-to-end tests for `cc config edit`.
//!
//! Uses a tempdir as `HOME` so XDG resolution targets a writable
//! location, and `EDITOR=/usr/bin/true` so the spawned editor exits
//! 0 without prompting.

use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::TempDir;

fn cc(tmp: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("cc").expect("cc binary");
    cmd.current_dir(tmp.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("COMMITCRAFTER_LOG")
        .env_remove("VISUAL")
        .env("HOME", tmp.path())
        .env("EDITOR", "/usr/bin/true");
    cmd
}

fn expected_global_path(tmp: &TempDir) -> PathBuf {
    tmp.path()
        .join(".config")
        .join("commitcrafter")
        .join("config.toml")
}

#[test]
fn config_edit_scaffolds_global_when_outside_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let global = expected_global_path(&tmp);
    assert!(!global.exists(), "precondition: global file does not exist");

    let out = cc(&tmp).args(["config", "edit"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    assert!(global.exists(), "scaffolded file should exist");
    let text = std::fs::read_to_string(&global).unwrap();
    assert!(text.starts_with("# commitcrafter configuration"));
    assert!(text.contains("[provider]"));
    assert!(text.contains("ANTHROPIC_API_KEY"));
}

#[test]
fn config_edit_does_not_clobber_existing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let global = expected_global_path(&tmp);
    std::fs::create_dir_all(global.parent().unwrap()).unwrap();
    std::fs::write(&global, "# my customizations\nkeep = true\n").unwrap();

    let out = cc(&tmp).args(["config", "edit"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let after = std::fs::read_to_string(&global).unwrap();
    assert_eq!(after, "# my customizations\nkeep = true\n");
}

#[test]
fn config_edit_global_flag_targets_global_even_inside_repo() {
    let tmp = tempfile::tempdir().unwrap();
    // Make tmp into a git repo so per-repo discovery succeeds.
    std::process::Command::new("git")
        .current_dir(tmp.path())
        .args(["init", "--quiet"])
        .status()
        .expect("git init")
        .success()
        .then_some(())
        .expect("git init succeeded");

    let global = expected_global_path(&tmp);
    let repo = tmp.path().join(".commitcrafter.toml");
    assert!(!global.exists());
    assert!(!repo.exists());

    let out = cc(&tmp)
        .args(["config", "edit", "--global"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // Global got created; per-repo did NOT.
    assert!(global.exists(), "global file should be scaffolded");
    assert!(!repo.exists(), "repo file must remain untouched");
}

#[test]
fn config_edit_repo_flag_errors_outside_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cc(&tmp)
        .args(["config", "edit", "--repo"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected failure when --repo is forced outside a repo",
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("not inside a git repository"),
        "stderr should explain the failure; got: {stderr}",
    );
}

#[test]
fn config_edit_inside_repo_targets_per_repo_file_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .current_dir(tmp.path())
        .args(["init", "--quiet"])
        .status()
        .expect("git init")
        .success()
        .then_some(())
        .expect("git init succeeded");

    let repo = tmp.path().join(".commitcrafter.toml");
    let global = expected_global_path(&tmp);

    let out = cc(&tmp).args(["config", "edit"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    assert!(
        repo.exists(),
        "repo file should be scaffolded inside a repo"
    );
    assert!(!global.exists(), "global must remain untouched");
}

#[test]
fn config_edit_template_round_trips_through_config_show() {
    // Scaffold, then run `cc config show` against the same HOME; the
    // freshly-scaffolded file should round-trip without flagging any
    // unknown keys (warnings go to stderr) and the resulting source
    // should be "global".
    let tmp = tempfile::tempdir().unwrap();
    let edit_out = cc(&tmp).args(["config", "edit"]).output().unwrap();
    assert!(
        edit_out.status.success(),
        "edit stderr: {}",
        String::from_utf8_lossy(&edit_out.stderr),
    );

    let show_out = cc(&tmp).args(["config", "show"]).output().unwrap();
    assert!(
        show_out.status.success(),
        "show stderr: {}",
        String::from_utf8_lossy(&show_out.stderr),
    );
    let stdout = String::from_utf8(show_out.stdout).unwrap();
    // The scaffolded global file should now be the source for every
    // leaf (since it duplicates the defaults but `Source::Global`
    // wins over `Source::Default` for every leaf the file mentions).
    assert!(
        stdout.contains("# source: global"),
        "expected `# source: global` in:\n{stdout}",
    );
    // And no unknown-key warnings.
    let stderr = String::from_utf8(show_out.stderr).unwrap();
    assert!(
        !stderr.contains("unknown config key"),
        "scaffold should not warn about unknown keys; got: {stderr}",
    );
}
