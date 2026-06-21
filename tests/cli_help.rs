use assert_cmd::Command;
use predicates::str::contains;

fn cc() -> Command {
    Command::cargo_bin("cc").expect("cc binary should build")
}

#[test]
fn help_lists_every_v01_flag() {
    let assert = cc().arg("--help").assert().success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    for needle in [
        "--yes",
        "--clipboard",
        "--generate",
        "--exclude",
        "--type",
        "--prompt",
        "--no-verify",
        "--print",
        "--provider",
        "--model",
        "--all",
        "--no-color",
        "--set",
    ] {
        assert!(out.contains(needle), "--help missing flag: {needle}\n{out}");
    }
}

#[test]
fn help_lists_every_v01_subcommand() {
    let assert = cc().arg("--help").assert().success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    for needle in [
        "setup",
        "init",
        "doctor",
        "config",
        "providers",
        "history",
        "forget",
    ] {
        assert!(
            out.contains(needle),
            "--help missing subcommand: {needle}\n{out}"
        );
    }
}

#[test]
fn version_prints_crate_version() {
    cc().arg("--version")
        .assert()
        .success()
        .stdout(contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn yes_and_clipboard_are_rejected_together() {
    cc().args(["-y", "-c"]).assert().failure();
}

#[test]
fn forget_without_target_fails() {
    cc().arg("forget").assert().failure();
}

#[test]
fn config_show_subsubcommand_parses() {
    cc().args(["config", "show", "--help"]).assert().success();
}
