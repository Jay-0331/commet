//! End-to-end tests for `cc config show`.
//!
//! Drives the `cc` binary via `assert_cmd` with a tempdir as `HOME`
//! and a non-repo `cwd` so no global or per-repo file exists; the
//! tests then assert that `--set` overrides flow through to the
//! rendered output.

use assert_cmd::Command;
use tempfile::TempDir;

fn cc(tmp: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("commet").expect("commet binary");
    cmd.current_dir(tmp.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("COMMET_LOG")
        .env("HOME", tmp.path());
    cmd
}

#[test]
fn config_show_emits_default_toml_when_no_files_present() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cc(&tmp).args(["config", "show"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();

    // Every block header from the schema appears.
    for header in [
        "[provider]",
        "[providers.anthropic]",
        "[style]",
        "[learning]",
        "[git]",
        "[ui]",
    ] {
        assert!(stdout.contains(header), "missing {header} in:\n{stdout}");
    }
    // Every leaf is annotated.
    assert!(
        stdout.contains("# source: default"),
        "no default sources:\n{stdout}"
    );
}

#[test]
fn config_show_reflects_set_overrides_in_output() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cc(&tmp)
        .args([
            "--set",
            "style.subject_max_len=42",
            "--set",
            "provider.default=openai",
            "config",
            "show",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();

    let subject_line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("subject_max_len ="))
        .unwrap_or_else(|| panic!("subject_max_len missing:\n{stdout}"));
    assert!(subject_line.contains("42"));
    assert!(subject_line.contains("# source: --set"));

    let default_line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("default ="))
        .unwrap_or_else(|| panic!("default missing:\n{stdout}"));
    assert!(default_line.contains("\"openai\""));
    assert!(default_line.contains("# source: --set"));
}

#[test]
fn config_show_json_is_parseable_and_keyed_by_path() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cc(&tmp)
        .args(["--set", "ui.theme=dracula", "config", "show", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    let obj = parsed.as_object().expect("top-level object");

    // The leaf we overrode should reflect the new value + source.
    let theme = obj.get("ui.theme").expect("ui.theme present");
    assert_eq!(theme["source"].as_str(), Some("--set"));
    assert_eq!(theme["value"].as_str(), Some("dracula"));

    // An untouched leaf should still appear, sourced from defaults.
    let scope = obj.get("learning.scope").expect("learning.scope present");
    assert_eq!(scope["source"].as_str(), Some("default"));
}

#[test]
fn config_show_propagates_invalid_set_path_as_error() {
    let tmp = tempfile::tempdir().unwrap();
    let out = cc(&tmp)
        .args(["--set", "style.subjct_max_len=10", "config", "show"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected non-zero exit; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("unknown --set path"));
    assert!(
        stderr.contains("style.subject_max_len"),
        "no did-you-mean: {stderr}"
    );
}
