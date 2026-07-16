//! `commet setup` — first-run global configuration bootstrap.

use std::fs;
use std::path::Path;

use crate::cli::{DoctorArgs, SetupArgs};
use crate::config::{Config, discover, edit};
use crate::error::{Error, Result};
use crate::provider;
use crate::tui::{self, SetupAction, Theme};

use super::doctor;

const PROVIDER_ENV: &str = "COMMET_PROVIDER";

pub fn run(args: &SetupArgs, cwd: &Path) -> Result<()> {
    let path = discover::global_config_path().ok_or_else(|| {
        Error::Config("no global config path available (set $XDG_CONFIG_HOME or $HOME)".into())
    })?;
    refuse_existing(&path, args.force)?;

    let provider = if args.noninteractive {
        provider_from_env()?.unwrap_or_else(|| "anthropic".into())
    } else {
        let initial = "anthropic";
        let ui = &Config::default().ui;
        let theme = Theme::from_config(ui, tui::color_cap(ui.color, false))
            .map_err(|err| Error::Config(err.to_string()))?;
        match tui::run_setup(&path, initial, theme)? {
            SetupAction::Select(provider) => provider,
            SetupAction::Quit => return Err(Error::UserAbort),
        }
    };

    write_config(&path, &provider, args.force)?;

    let doctor_result = doctor::run(
        &DoctorArgs {
            full: false,
            json: false,
        },
        cwd,
    );
    print_summary(&path, &provider);
    doctor_result
}

fn refuse_existing(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Err(Error::Config(format!(
            "global config already exists at {} (run `commet config edit --global` or rerun setup with --force)",
            path.display()
        )));
    }
    Ok(())
}

fn provider_from_env() -> Result<Option<String>> {
    let Some(value) = std::env::var(PROVIDER_ENV)
        .ok()
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    validate_provider(&value)?;
    Ok(Some(value))
}

fn validate_provider(name: &str) -> Result<()> {
    if provider::registry().contains_key(name) {
        Ok(())
    } else {
        Err(Error::Config(format!(
            "unknown provider `{name}` in ${PROVIDER_ENV} (expected anthropic, openai, openrouter, or ollama)"
        )))
    }
}

fn write_config(path: &Path, provider: &str, force: bool) -> Result<()> {
    validate_provider(provider)?;
    refuse_existing(path, force)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| Error::Config(format!("create {}: {err}", parent.display())))?;
    }
    let mut config = Config::default();
    config.provider.default = provider.into();
    let body = format!("{}{}", edit::TEMPLATE_HEADER, config.to_toml_string()?);
    fs::write(path, body).map_err(|err| Error::Config(format!("write {}: {err}", path.display())))
}

fn print_summary(path: &Path, provider_name: &str) {
    println!("\nSetup complete");
    println!("  Config: {}", path.display());
    println!("  Edit:   commet config edit --global");
    if let Some(provider) = provider::registry().get(provider_name)
        && let Some(key) = provider.key_env_var()
        && std::env::var(key)
            .map(|value| value.is_empty())
            .unwrap_or(true)
    {
        println!("  Key:    export {key}=<your key>");
    }
    println!("  Check:  commet doctor");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_config_selects_provider_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commet/config.toml");
        write_config(&path, "openrouter", false).unwrap();
        let text = fs::read_to_string(path).unwrap();
        let config = Config::from_toml_str(&text).unwrap();
        assert_eq!(config.provider.default, "openrouter");
        assert!(text.contains("[providers.anthropic]"));
        assert!(text.contains("[providers.ollama]"));
    }

    #[test]
    fn existing_config_requires_force_and_force_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "keep me").unwrap();
        assert!(write_config(&path, "ollama", false).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
        write_config(&path, "ollama", true).unwrap();
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .contains("default = \"ollama\"")
        );
    }

    #[test]
    fn invalid_provider_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = write_config(&dir.path().join("config.toml"), "bogus", false).unwrap_err();
        assert!(matches!(err, Error::Config(message) if message.contains("unknown provider")));
    }
}
