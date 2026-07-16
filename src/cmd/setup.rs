//! `commet setup` — first-run global configuration bootstrap.

use std::fs;
use std::path::Path;

use crate::cli::{DoctorArgs, SetupArgs};
use crate::config::{Config, discover};
use crate::error::{Error, Result};
use crate::provider;
use crate::tui::{self, SetupAction, SetupReport, Theme};

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
        let action = tui::run_setup(
            &path,
            initial,
            theme,
            |provider| write_config(&path, provider, args.force),
            |provider| {
                let report = doctor::collect(
                    &DoctorArgs {
                        full: false,
                        json: false,
                    },
                    cwd,
                );
                Ok(SetupReport {
                    results: report.results,
                    hints: summary_lines(&path, provider),
                })
            },
        )?;
        return match action {
            SetupAction::Complete {
                doctor_failed: true,
            } => Err(Error::Doctor),
            SetupAction::Complete {
                doctor_failed: false,
            } => Ok(()),
            SetupAction::Quit => return Err(Error::UserAbort),
        };
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
    let body = starter_config(provider);
    fs::write(path, body).map_err(|err| Error::Config(format!("write {}: {err}", path.display())))
}

fn print_summary(path: &Path, provider_name: &str) {
    println!("\nSetup complete");
    for line in summary_lines(path, provider_name) {
        println!("  {line}");
    }
}

fn summary_lines(path: &Path, provider_name: &str) -> Vec<String> {
    let mut lines = vec![
        format!("Config: {}", path.display()),
        "Edit:   commet config edit --global".into(),
    ];
    if let Some(provider) = provider::registry().get(provider_name)
        && let Some(key) = provider.key_env_var()
        && std::env::var(key)
            .map(|value| value.is_empty())
            .unwrap_or(true)
    {
        lines.push(format!("Key:    export {key}=<your key>"));
    }
    lines.push("Check:  commet doctor".into());
    lines
}

fn starter_config(provider: &str) -> String {
    COMMENTED_TEMPLATE.replace("__PROVIDER__", provider)
}

const COMMENTED_TEMPLATE: &str = r##"# commet configuration
# API keys are read only from ANTHROPIC_API_KEY, OPENAI_API_KEY, or OPENROUTER_API_KEY.

[provider]
# Provider used unless --provider overrides it for one run.
default = "__PROVIDER__"

[providers.anthropic]
# Anthropic Messages API model id.
model = "claude-sonnet-4-6"
# Maximum generated tokens per candidate.
max_tokens = 1024
# Sampling temperature (lower is more deterministic).
temperature = 0.2
# Request timeout in seconds.
timeout_secs = 60
# Retries after rate limits, server errors, or transport failures.
max_retries = 2

[providers.openai]
# OpenAI chat-completions model id.
model = "gpt-4o-mini"
# Maximum generated tokens per candidate.
max_tokens = 1024
# Sampling temperature.
temperature = 0.2
# Request timeout in seconds.
timeout_secs = 60
# Retry budget.
max_retries = 2

[providers.openrouter]
# OpenRouter API base URL.
endpoint = "https://openrouter.ai/api/v1"
# OpenRouter model id.
model = "anthropic/claude-sonnet-4"
# Maximum generated tokens per candidate.
max_tokens = 1024
# Sampling temperature.
temperature = 0.2
# Optional app attribution URL.
http_referer = ""
# App name sent to OpenRouter.
x_title = "commet"
# Request timeout in seconds.
timeout_secs = 60
# Retry budget.
max_retries = 2

[providers.ollama]
# Local Ollama server URL.
endpoint = "http://localhost:11434"
# Installed Ollama model name.
model = "llama3.1:8b"
# Request timeout in seconds (model loading can be slow).
timeout_secs = 60
# Retry budget.
max_retries = 2

[style]
# Message format: plain, conventional, conventional+body, gitmoji, subject+body, or custom.
format = "plain"
# Maximum subject length.
subject_max_len = 72
# Body wrapping column.
body_wrap = 72
# Permit a commit-message body.
include_body = true
# Conventional-commit types available to the prompt.
allowed_types = ["feat", "fix", "refactor", "docs", "test", "chore", "perf", "ci", "build", "style"]
# Optional conventional scopes; empty lets the model infer them.
allowed_scopes = []
# Handwritten examples supplied to the prompt.
examples = ["feat(auth): add OAuth device flow", "fix(parser): handle trailing comma in arrays"]
# Number of candidates generated by default.
generate = 1
# Extra instructions appended to every prompt.
extra_prompt = ""

[style.custom]
# Complete system prompt used when format = custom.
system_prompt = ""
# Optional output template used when format = custom.
template = ""

[learning]
# Learn from accepted commit messages.
enabled = true
# Store scope: off, repo, global, or repo+global.
scope = "repo+global"
# Maximum learned examples injected into a prompt.
max_examples = 5
# Persist source diffs with learning records (privacy-sensitive).
store_diffs = false
# Optional custom store path; empty uses XDG/repo defaults.
store_path = ""
# Add the per-repo history directory to .gitignore automatically.
auto_gitignore = true

[git]
# Undo paths staged by commet if the user aborts.
auto_unstage_on_abort = true
# Paths hidden from the model by default (they remain staged).
ignore_paths = ["package-lock.json", "*.lock", "dist/**"]
# Maximum diff bytes sent to the model.
diff_max_bytes = 102400

[ui]
# Theme: default, mono, dracula, solarized-dark, solarized-light, or custom.
theme = "default"
# Color policy: auto, always, or never.
color = "auto"
# Use Unicode glyphs when true.
unicode = true

[ui.custom]
# Primary foreground color.
fg = "white"
# Background color.
bg = "reset"
# Interactive accent color.
accent = "#7aa2f7"
# Success status color.
success = "green"
# Warning status color.
warning = "yellow"
# Failure status color.
error = "red"
# Muted/help text color.
muted = "bright_black"
# Added-diff color.
diff_add = "green"
# Deleted-diff color.
diff_del = "red"
# Diff metadata color.
diff_meta = "cyan"
# Widget border color.
border = "bright_black"
"##;

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
        let mut expected = Config::default();
        expected.provider.default = "openrouter".into();
        assert_eq!(config, expected);
        assert!(text.contains("[providers.anthropic]"));
        assert!(text.contains("[providers.ollama]"));
        for (index, line) in text.lines().enumerate() {
            if line.contains('=') && !line.trim_start().starts_with('#') {
                assert!(
                    text.lines()
                        .nth(index.saturating_sub(1))
                        .unwrap_or("")
                        .starts_with('#'),
                    "key lacks explanatory comment: {line}"
                );
            }
        }
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
