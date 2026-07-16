//! `commet doctor` — probe the current environment, render all shared
//! preflight checks, and return exit code 5 when any check hard-fails.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cli::DoctorArgs;
use crate::config::{Config, Layered, discover};
use crate::doctor::{self, CheckCtx, CheckResult, Status};
use crate::error::{Error, Result};
use crate::{git, learning, provider, tui};

use super::providers;

/// Probe the real process environment and run the command.
pub fn run(args: &DoctorArgs, cwd: &Path) -> Result<()> {
    let (config, config_error, repo_config_error) = load_config_snapshot(cwd);
    let repo_root = git::repo_root(cwd).ok();
    let registry = provider::registry();
    let provider_name = config.provider.default.clone();
    let selected = registry.get(provider_name.as_str());
    let key_env = selected.and_then(|provider| provider.key_env_var());
    let key_present = key_env
        .map(|name| std::env::var(name).is_ok_and(|value| !value.is_empty()))
        .unwrap_or(false);

    // Reachability is useful without `--full`; the latter's authenticated
    // one-token completion is implemented separately in #53. Avoid a network
    // probe when the provider/key checks have already made the result moot.
    let reachable = if selected.is_some() && (key_env.is_none() || key_present) {
        providers::selected_reachable(&config, &provider_name)
    } else {
        None
    };

    let store = learning::Store::open(config.learning.scope, repo_root.as_deref());
    let ctx = CheckCtx {
        git_version: git::version().ok(),
        in_repo: repo_root.is_some(),
        config_error,
        repo_config_error,
        provider: provider_name,
        provider_registered: selected.is_some(),
        key_env: key_env.map(str::to_owned),
        key_present,
        reachable,
        editor: std::env::var("VISUAL")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                std::env::var("EDITOR")
                    .ok()
                    .filter(|value| !value.is_empty())
            }),
        color: tui::color_cap(config.ui.color, false),
        // Clipboard dispatch is not implemented yet, so reporting an
        // available backend would promise functionality that does not exist.
        clipboard_available: false,
        store_writable: !store.is_enabled() || paths_writable(&store.paths()),
    };

    let smoke_result = args.full.then(|| match selected {
        Some(provider) if key_env.is_none() || key_present => {
            doctor::smoke(provider.as_ref(), provider_model(&config, &ctx.provider))
        }
        Some(_) => doctor::smoke_skipped("the provider API key is missing"),
        None => doctor::smoke_skipped("the configured provider is not registered"),
    });

    execute(&ctx, args, config.ui.unicode, smoke_result)
}

/// Render already-probed checks. Kept separate so failure/exit behavior is
/// tested without mutating process-global environment variables.
fn execute(
    ctx: &CheckCtx,
    args: &DoctorArgs,
    unicode: bool,
    smoke_result: Option<CheckResult>,
) -> Result<()> {
    let mut results = doctor::run_all(ctx);
    if let Some(result) = smoke_result {
        results.push(result);
    }
    let out = if args.json {
        render_json(&results)
    } else {
        render_human(&results, unicode)
    };
    print!("{out}");
    if !out.ends_with('\n') {
        println!();
    }

    if results.iter().any(CheckResult::is_fail) {
        Err(Error::Doctor)
    } else {
        Ok(())
    }
}

fn provider_model<'a>(config: &'a Config, provider: &str) -> &'a str {
    match provider {
        "anthropic" => &config.providers.anthropic.model,
        "openai" => &config.providers.openai.model,
        "openrouter" => &config.providers.openrouter.model,
        "ollama" => &config.providers.ollama.model,
        _ => "",
    }
}

/// Load global and repo config independently so a malformed repo file can be
/// reported by its dedicated check while the remaining probes still run.
fn load_config_snapshot(cwd: &Path) -> (Config, Option<String>, Option<String>) {
    let global_path = discover::global_config_path().filter(|path| path.exists());
    let repo_path = discover::repo_config_path_in(cwd).filter(|path| path.exists());

    let mut global_error = None;
    let mut repo_error = None;
    let mut layered = Layered::new();

    if let Some(path) = global_path {
        match config_value(&path) {
            Ok(value) => {
                let validation = Layered::new()
                    .with_global_value(path.clone(), value.clone())
                    .load();
                match validation {
                    Ok(_) => layered = layered.with_global_value(path, value),
                    Err(err) => global_error = Some(err.to_string()),
                }
            }
            Err(err) => {
                global_error = Some(err.to_string());
            }
        }
    }
    if let Some(path) = repo_path {
        match config_value(&path) {
            Ok(value) => {
                let validation = Layered::new()
                    .with_repo_value(path.clone(), value.clone())
                    .load();
                match validation {
                    Ok(_) => layered = layered.with_repo_value(path, value),
                    Err(err) => repo_error = Some(err.to_string()),
                }
            }
            Err(err) => repo_error = Some(err.to_string()),
        }
    }

    let config = layered
        .load()
        .map(|loaded| loaded.config)
        .unwrap_or_default();
    (config, global_error, repo_error)
}

fn config_value(path: &Path) -> Result<toml::Value> {
    let text = fs::read_to_string(path)
        .map_err(|err| Error::Config(format!("read {}: {err}", path.display())))?;
    toml::from_str(&text).map_err(|err| Error::Config(format!("parse {}: {err}", path.display())))
}

fn paths_writable(paths: &[PathBuf]) -> bool {
    paths.iter().all(|path| {
        let mut candidate = path.as_path();
        while !candidate.exists() {
            let Some(parent) = candidate.parent() else {
                return false;
            };
            candidate = parent;
        }
        fs::metadata(candidate)
            .map(|metadata| !metadata.permissions().readonly())
            .unwrap_or(false)
    })
}

fn render_human(results: &[CheckResult], unicode: bool) -> String {
    let mut out = String::new();
    for result in results {
        let (glyph, message) = match &result.status {
            Status::Ok(message) => (if unicode { "✓" } else { "OK" }, message),
            Status::Warn(message) => (if unicode { "⚠" } else { "!!" }, message),
            Status::Fail(message) => (if unicode { "✗" } else { "XX" }, message),
        };
        out.push_str(&format!("{glyph} {}: {message}\n", result.name));
        if let Some(hint) = &result.fix_hint {
            out.push_str(&format!("  fix: {hint}\n"));
        }
    }
    out
}

#[derive(Serialize)]
struct JsonResult<'a> {
    name: &'a str,
    status: &'static str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix_hint: Option<&'a str>,
}

fn render_json(results: &[CheckResult]) -> String {
    let rows: Vec<JsonResult<'_>> = results
        .iter()
        .map(|result| {
            let (status, message) = match &result.status {
                Status::Ok(message) => ("ok", message.as_str()),
                Status::Warn(message) => ("warn", message.as_str()),
                Status::Fail(message) => ("fail", message.as_str()),
            };
            JsonResult {
                name: result.name,
                status,
                message,
                fix_hint: result.fix_hint.as_deref(),
            }
        })
        .collect();
    serde_json::to_string_pretty(&rows).expect("doctor results serialize to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::ColorCap;

    fn healthy() -> CheckCtx {
        CheckCtx {
            git_version: Some("git version 2.44.0".into()),
            in_repo: true,
            config_error: None,
            repo_config_error: None,
            provider: "ollama".into(),
            provider_registered: true,
            key_env: None,
            key_present: false,
            reachable: Some(true),
            editor: Some("vim".into()),
            color: ColorCap::Ansi256,
            clipboard_available: true,
            store_writable: true,
        }
    }

    fn args(json: bool) -> DoctorArgs {
        DoctorArgs { full: false, json }
    }

    #[test]
    fn human_output_includes_failure_and_fix_hint() {
        let mut ctx = healthy();
        ctx.key_env = Some("TEST_KEY".into());
        ctx.key_present = false;
        let results = crate::doctor::run_all(&ctx);
        let out = render_human(&results, true);
        assert!(out.contains("✗ provider API key: $TEST_KEY is not set"));
        assert!(out.contains("fix: export TEST_KEY=<your key>"));
    }

    #[test]
    fn ascii_output_contains_no_unicode_glyphs() {
        let out = render_human(&crate::doctor::run_all(&healthy()), false);
        assert!(out.starts_with("OK git available"));
        assert!(!out.contains(['✓', '⚠', '✗']));
    }

    #[test]
    fn json_is_stable_and_machine_readable() {
        let value: serde_json::Value =
            serde_json::from_str(&render_json(&crate::doctor::run_all(&healthy()))).unwrap();
        assert_eq!(value[0]["name"], "git available");
        assert_eq!(value[0]["status"], "ok");
        assert_eq!(value.as_array().unwrap().len(), 11);
    }

    #[test]
    fn failures_map_to_doctor_exit_error() {
        let mut ctx = healthy();
        ctx.git_version = None;
        assert!(matches!(
            execute(&ctx, &args(true), true, None),
            Err(Error::Doctor)
        ));
        assert_eq!(Error::Doctor.exit_code(), 5);
    }

    #[test]
    fn warnings_still_succeed() {
        let mut ctx = healthy();
        ctx.editor = None;
        assert!(execute(&ctx, &args(true), true, None).is_ok());
    }

    #[test]
    fn full_result_is_appended_to_json() {
        let mut full_args = args(true);
        full_args.full = true;
        let results = doctor::run_all(&healthy());
        let mut with_smoke = results.clone();
        with_smoke.push(doctor::smoke_skipped("test prerequisite"));
        let value: serde_json::Value = serde_json::from_str(&render_json(&with_smoke)).unwrap();
        assert_eq!(value.as_array().unwrap().len(), 12);
        assert_eq!(value[11]["name"], doctor::SMOKE_CHECK_NAME);
        assert_eq!(value[11]["status"], "warn");

        assert!(
            execute(
                &healthy(),
                &full_args,
                true,
                Some(with_smoke.pop().unwrap())
            )
            .is_ok()
        );
    }

    #[test]
    fn malformed_repo_config_is_captured_separately() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        fs::write(dir.path().join(".commet.toml"), "not = [valid").unwrap();
        let (_, global, repo) = load_config_snapshot(dir.path());
        assert!(global.is_none());
        assert!(repo.as_deref().is_some_and(|error| error.contains("parse")));
    }

    #[test]
    fn invalid_typed_repo_config_is_captured_separately() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        fs::write(dir.path().join(".commet.toml"), "[ui]\ncolor = \"bogus\"\n").unwrap();
        let (_, global, repo) = load_config_snapshot(dir.path());
        assert!(global.is_none());
        assert!(repo.as_deref().is_some_and(|error| error.contains("color")));
    }

    #[test]
    fn writable_paths_walk_up_to_existing_parent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(paths_writable(&[dir.path().join("new/dir/history.jsonl")]));
    }
}
