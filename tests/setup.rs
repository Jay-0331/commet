use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn noninteractive_setup_writes_config_and_doctor_passes() {
    let dir = tempfile::tempdir().unwrap();
    let config_home = dir.path().join("config");
    let state_home = dir.path().join("state");

    let mut setup = Command::cargo_bin("commet").unwrap();
    setup
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("COMMET_PROVIDER", "ollama")
        .args(["setup", "--noninteractive"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Setup complete"))
        .stdout(predicate::str::contains("commet doctor"));

    let config = config_home.join("commet/config.toml");
    assert!(config.exists());
    assert!(
        std::fs::read_to_string(config)
            .unwrap()
            .contains("default = \"ollama\"")
    );

    let mut doctor = Command::cargo_bin("commet").unwrap();
    doctor
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_STATE_HOME", state_home)
        .args(["doctor", "--json"])
        .assert()
        .success();
}

#[test]
fn setup_refuses_existing_config_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let config_home = dir.path().join("config");
    let config = config_home.join("commet/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "keep me").unwrap();

    let mut setup = Command::cargo_bin("commet").unwrap();
    setup
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", config_home)
        .args(["setup", "--noninteractive"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("already exists"));
    assert_eq!(std::fs::read_to_string(config).unwrap(), "keep me");
}
