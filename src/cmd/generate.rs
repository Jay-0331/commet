//! The default (no-subcommand) flow: generate a commit message from the
//! staged diff and either print it or commit it.
//!
//! Pipeline: read the staged diff → shrink it to the config byte cap
//! (dropping `-x`/`[git].ignore_paths` globs) → build the prompt from
//! `[style]` → call the configured provider → print candidates
//! (`--print`, or the no-flag default) or commit the first one (`-y`).
//!
//! The interactive TUI preview (file picker / candidate picker) is a
//! later milestone; until it lands, the no-flag default prints the
//! message with a hint rather than committing, so nothing happens to
//! the repo without `-y`.

use std::io::Write;
use std::path::Path;

use crate::cli::{self, GenerateOpts};
use crate::config::Config;
use crate::config::schema::{self, Style};
use crate::error::{Error, Result};
use crate::git;
use crate::prompt::{self, GenOpts};
use crate::provider::registry;

/// Fallback generation params for providers whose config block carries
/// no `max_tokens`/`temperature` (Ollama).
const DEFAULT_MAX_TOKENS: u32 = 1024;
const DEFAULT_TEMPERATURE: f32 = 0.2;

/// Run the default generate flow in `cwd`.
pub fn run(config: &Config, opts: &GenerateOpts, cwd: &Path) -> Result<()> {
    if opts.clipboard {
        return Err(Error::Config("--clipboard/-c is not supported yet".into()));
    }
    if opts.all {
        return Err(Error::Config(
            "--all is not supported yet — stage changes with `git add` first".into(),
        ));
    }

    let provider_name = opts
        .provider
        .clone()
        .unwrap_or_else(|| config.provider.default.clone());

    let reg = registry();
    let provider = reg.get(provider_name.as_str()).ok_or_else(|| {
        Error::Config(format!(
            "unknown provider `{provider_name}` (expected anthropic, openai, openrouter, or ollama)"
        ))
    })?;

    // Fail early with a clear message if the key is missing, rather than
    // building the whole request first.
    if let Some(var) = provider.key_env_var()
        && std::env::var(var).map(|v| v.is_empty()).unwrap_or(true)
    {
        return Err(Error::Provider(format!(
            "{provider_name}: missing API key — set ${var}"
        )));
    }

    let diff = git::diff_staged(cwd)?;
    if diff.trim().is_empty() {
        return Err(Error::Git(
            "nothing staged — stage changes with `git add` before generating".into(),
        ));
    }

    let entries = git::status_porcelain(cwd)?;
    let files: Vec<String> = entries
        .iter()
        .map(|e| e.path.display().to_string())
        .collect();

    let ignore = git::merge_ignore_globs(&opts.exclude, &config.git.ignore_paths);
    let shrunk = git::truncate_diff(&diff, &entries, config.git.diff_max_bytes as usize, &ignore)?;

    let (model_default, max_tokens, temperature) = provider_gen_params(config, &provider_name)?;
    let model = opts.model.clone().unwrap_or(model_default);
    let n = resolve_count(opts.count, config.style.generate);

    let style = Style {
        format: resolve_format(opts.format, config.style.format),
        ..config.style.clone()
    };
    let gen_opts = GenOpts {
        model,
        max_tokens,
        temperature,
        n,
        extra_prompt: resolve_extra(opts.prompt.as_deref(), &config.style.extra_prompt),
    };

    let request = prompt::build(&style, &shrunk, &files, &gen_opts);
    let candidates = provider
        .generate(&request)
        .map_err(|e| Error::Provider(e.to_string()))?;
    if candidates.is_empty() {
        return Err(Error::Provider("provider returned no candidates".into()));
    }

    if opts.print {
        print!("{}", render_candidates(&candidates));
        if !render_candidates(&candidates).ends_with('\n') {
            println!();
        }
        return Ok(());
    }

    if opts.yes {
        return commit(cwd, &candidates[0], opts.no_verify);
    }

    // No flag and no TUI yet: show the message, don't touch the repo.
    print!("{}", render_candidates(&candidates));
    if !render_candidates(&candidates).ends_with('\n') {
        println!();
    }
    eprintln!("\n(preview only — re-run with -y to commit, or --print for plain output)");
    Ok(())
}

/// Resolve the effective message format: the `-t/--type` flag wins over
/// the configured `[style].format`.
fn resolve_format(
    cli_format: Option<cli::MessageFormat>,
    config_format: schema::MessageFormat,
) -> schema::MessageFormat {
    match cli_format {
        Some(f) => map_format(f),
        None => config_format,
    }
}

/// Map the clap `-t/--type` enum to the config enum (distinct types).
fn map_format(f: cli::MessageFormat) -> schema::MessageFormat {
    match f {
        cli::MessageFormat::Plain => schema::MessageFormat::Plain,
        cli::MessageFormat::Conventional => schema::MessageFormat::Conventional,
        cli::MessageFormat::ConventionalBody => schema::MessageFormat::ConventionalBody,
        cli::MessageFormat::Gitmoji => schema::MessageFormat::Gitmoji,
        cli::MessageFormat::SubjectBody => schema::MessageFormat::SubjectBody,
        cli::MessageFormat::Custom => schema::MessageFormat::Custom,
    }
}

/// Candidate count: `-g/--generate` wins over `[style].generate`,
/// clamped to the supported 1..=5 range.
fn resolve_count(cli_count: Option<u32>, config_count: u32) -> u8 {
    cli_count.unwrap_or(config_count).clamp(1, 5) as u8
}

/// Extra system-prompt text: `-p/--prompt` wins over
/// `[style].extra_prompt`; empty/whitespace yields `None`.
fn resolve_extra(cli_prompt: Option<&str>, config_extra: &str) -> Option<String> {
    if let Some(p) = cli_prompt
        && !p.trim().is_empty()
    {
        return Some(p.to_string());
    }
    if !config_extra.trim().is_empty() {
        return Some(config_extra.to_string());
    }
    None
}

/// Model / max_tokens / temperature for the named provider. Ollama's
/// config block has no sampling knobs, so it uses the defaults.
fn provider_gen_params(config: &Config, name: &str) -> Result<(String, u32, f32)> {
    let p = &config.providers;
    let params = match name {
        "anthropic" => (
            p.anthropic.model.clone(),
            p.anthropic.max_tokens,
            p.anthropic.temperature,
        ),
        "openai" => (
            p.openai.model.clone(),
            p.openai.max_tokens,
            p.openai.temperature,
        ),
        "openrouter" => (
            p.openrouter.model.clone(),
            p.openrouter.max_tokens,
            p.openrouter.temperature,
        ),
        "ollama" => (
            p.ollama.model.clone(),
            DEFAULT_MAX_TOKENS,
            DEFAULT_TEMPERATURE,
        ),
        other => {
            return Err(Error::Config(format!("unknown provider `{other}`")));
        }
    };
    Ok(params)
}

/// Render candidates for stdout: the bare message for a single
/// candidate, or numbered blocks for several.
fn render_candidates(candidates: &[String]) -> String {
    if candidates.len() == 1 {
        return candidates[0].clone();
    }
    candidates
        .iter()
        .enumerate()
        .map(|(i, c)| format!("--- candidate {} ---\n{}", i + 1, c))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Commit `message` via `git commit -F <tempfile>` so the user's
/// `commit.template`, hooks, and signing behave exactly as a normal
/// commit. Honors `--no-verify`.
fn commit(cwd: &Path, message: &str, no_verify: bool) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(message.as_bytes())?;
    git::commit(cwd, file.path(), no_verify)?;

    let subject = message.lines().next().unwrap_or_default();
    println!("Committed: {subject}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_flag_overrides_config() {
        assert_eq!(
            resolve_format(
                Some(cli::MessageFormat::Gitmoji),
                schema::MessageFormat::Plain
            ),
            schema::MessageFormat::Gitmoji
        );
        assert_eq!(
            resolve_format(None, schema::MessageFormat::Conventional),
            schema::MessageFormat::Conventional
        );
    }

    #[test]
    fn count_clamps_to_one_through_five() {
        assert_eq!(resolve_count(Some(3), 1), 3);
        assert_eq!(resolve_count(None, 2), 2);
        assert_eq!(resolve_count(Some(99), 1), 5); // clamp high
        assert_eq!(resolve_count(None, 0), 1); // clamp low
    }

    #[test]
    fn extra_prompt_prefers_cli_then_config() {
        assert_eq!(resolve_extra(Some("cli"), "cfg"), Some("cli".to_string()));
        assert_eq!(resolve_extra(None, "cfg"), Some("cfg".to_string()));
        assert_eq!(resolve_extra(Some("  "), "cfg"), Some("cfg".to_string()));
        assert_eq!(resolve_extra(None, ""), None);
    }

    #[test]
    fn provider_params_pull_from_config_block() {
        let mut config = Config::default();
        config.providers.anthropic.model = "claude-x".into();
        config.providers.anthropic.max_tokens = 999;
        let (model, max_tokens, _) = provider_gen_params(&config, "anthropic").unwrap();
        assert_eq!(model, "claude-x");
        assert_eq!(max_tokens, 999);

        // Ollama uses the sampling defaults.
        let (_, mt, temp) = provider_gen_params(&config, "ollama").unwrap();
        assert_eq!(mt, DEFAULT_MAX_TOKENS);
        assert_eq!(temp, DEFAULT_TEMPERATURE);

        assert!(provider_gen_params(&config, "bogus").is_err());
    }

    #[test]
    fn render_candidates_single_vs_multi() {
        assert_eq!(render_candidates(&["only".into()]), "only");
        let multi = render_candidates(&["a".into(), "b".into()]);
        assert!(multi.contains("--- candidate 1 ---\na"));
        assert!(multi.contains("--- candidate 2 ---\nb"));
    }
}
