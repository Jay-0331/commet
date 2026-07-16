//! The default (no-subcommand) flow: select changed files, generate a
//! commit message from the staged diff, then preview and commit it.
//!
//! Interactive pipeline: file picker → stage the selection → shrink the
//! diff to the config byte cap → call the provider → preview → commit.
//! Explicit `--print` / `-y` invocations keep operating on the user's
//! already-staged index so they remain script-friendly.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::{self, GenerateOpts};
use crate::config::Config;
use crate::config::schema::{self, Style};
use crate::error::{Error, Result};
use crate::git;
use crate::learning::{LearningRecord, Store};
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

    let interactive = !opts.print && !opts.yes && std::io::stdout().is_terminal();
    let theme = if interactive {
        let cap = crate::tui::color_cap(config.ui.color, opts.no_color);
        Some(
            crate::tui::Theme::from_config(&config.ui, cap)
                .map_err(|error| Error::Config(error.to_string()))?,
        )
    } else {
        None
    };

    // In the default terminal flow, the file picker is the first visible
    // interaction. Keep its staging guard alive through provider generation
    // and preview so every error/abort restores this session's additions.
    let mut stage_tracker = None;
    if interactive {
        let entries = git::status_porcelain(cwd)?;
        if entries.is_empty() {
            return Err(Error::Git(
                "working tree is clean — no files to select".into(),
            ));
        }
        let state =
            crate::tui::FilePickerState::from_status(cwd, entries, &config.git.ignore_paths)?;
        let picked = crate::tui::run_file_picker(state, theme.expect("interactive theme"))?;
        let crate::tui::FilePickerOutcome::Selected(paths) = picked else {
            return Err(Error::UserAbort);
        };

        let already_staged = git::staged_paths(cwd)?;
        let path_refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        let mut tracker =
            git::StageTracker::new(cwd.to_path_buf(), config.git.auto_unstage_on_abort);
        tracker.stage_preserving(&path_refs, &already_staged)?;
        stage_tracker = Some(tracker);
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

    let staged_paths = git::staged_paths(cwd)?;
    let entries: Vec<_> = git::status_porcelain(cwd)?
        .into_iter()
        .filter(|entry| staged_paths.iter().any(|path| path == &entry.path))
        .collect();
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
        examples: learned_examples(config, cwd, style.format),
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
        commit(cwd, &candidates[0], opts.no_verify)?;
        record_accepted(
            config,
            cwd,
            &RecordCtx {
                provider: &provider_name,
                model: &gen_opts.model,
                format: style.format,
                candidates: &candidates,
                accepted_index: 0,
                files: &files,
                diff: &shrunk,
            },
        );
        return Ok(());
    }

    // No flag: interactive preview on a real terminal; otherwise (piped
    // output, CI) fall back to printing so scripting still works.
    if interactive {
        return interactive_preview(
            &candidates,
            PreviewSession {
                config,
                opts,
                cwd,
                provider: provider.as_ref(),
                request: &request,
                generation: Ctx {
                    provider: &provider_name,
                    model: &gen_opts.model,
                    temperature: gen_opts.temperature,
                    format: style.format,
                    files: &files,
                    diff: &shrunk,
                },
                theme: theme.expect("interactive theme"),
                stage_tracker,
            },
        );
    }

    print!("{}", render_candidates(&candidates));
    if !render_candidates(&candidates).ends_with('\n') {
        println!();
    }
    eprintln!("\n(preview only — re-run with -y to commit, or --print for plain output)");
    Ok(())
}

/// Context threaded into the interactive preview and its learning record.
struct Ctx<'a> {
    provider: &'a str,
    model: &'a str,
    temperature: f32,
    format: schema::MessageFormat,
    files: &'a [String],
    diff: &'a str,
}

/// Dependencies and owned staging guard for one preview session.
struct PreviewSession<'a> {
    config: &'a Config,
    opts: &'a GenerateOpts,
    cwd: &'a Path,
    provider: &'a dyn crate::provider::Provider,
    request: &'a crate::provider::GenerateRequest,
    generation: Ctx<'a>,
    theme: crate::tui::Theme,
    stage_tracker: Option<git::StageTracker>,
}

/// Run the interactive preview: accept → commit + record, quit →
/// user-abort, regenerate → re-query the provider, edit → `$EDITOR`.
fn interactive_preview(candidates: &[String], session: PreviewSession<'_>) -> Result<()> {
    let PreviewSession {
        config,
        opts,
        cwd,
        provider,
        request,
        generation: ctx,
        theme,
        stage_tracker,
    } = session;
    let state = crate::tui::PreviewState::new(
        candidates.to_vec(),
        ctx.provider,
        ctx.model,
        ctx.temperature,
        config.style.body_wrap as u16,
    );

    let outcome = crate::tui::run_preview(
        state,
        theme,
        || provider.generate(request).map_err(|e| e.to_string()),
        |text| edit_in_editor(text).map_err(|e| e.to_string()),
    )?;

    match outcome {
        crate::tui::PreviewOutcome::Accepted(acc) => {
            commit(cwd, &acc.message, opts.no_verify)?;
            if let Some(tracker) = stage_tracker {
                tracker.release();
            }
            record_accepted(
                config,
                cwd,
                &RecordCtx {
                    provider: ctx.provider,
                    model: ctx.model,
                    format: ctx.format,
                    candidates: &acc.candidates,
                    accepted_index: acc.index,
                    files: ctx.files,
                    diff: ctx.diff,
                },
            );
            Ok(())
        }
        crate::tui::PreviewOutcome::Aborted => {
            if let Some(tracker) = stage_tracker {
                tracker.abort()?;
            }
            Err(Error::UserAbort)
        }
    }
}

/// Write `text` to a tempfile, open `$EDITOR` on it, and return the
/// edited contents (trailing whitespace trimmed).
fn edit_in_editor(text: &str) -> Result<String> {
    let mut file = tempfile::Builder::new().suffix(".txt").tempfile()?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    crate::editor::spawn(file.path())?;
    let edited = std::fs::read_to_string(file.path())?;
    Ok(edited.trim_end().to_string())
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

/// The data recorded to the learning store on an accepted commit.
struct RecordCtx<'a> {
    provider: &'a str,
    model: &'a str,
    format: schema::MessageFormat,
    candidates: &'a [String],
    accepted_index: usize,
    files: &'a [String],
    diff: &'a str,
}

/// Append a [`LearningRecord`] for an accepted commit. Best-effort:
/// respects `[learning].enabled` + scope, and a write failure is logged
/// rather than surfaced — the commit already succeeded.
fn record_accepted(config: &Config, cwd: &Path, acc: &RecordCtx) {
    if !config.learning.enabled {
        return;
    }
    let repo_root = git::repo_root(cwd).ok();
    let store = Store::open(config.learning.scope, repo_root.as_deref());
    if !store.is_enabled() {
        return;
    }

    let repo = repo_root
        .as_deref()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let branch = git::current_branch(cwd).unwrap_or_else(|_| "HEAD".into());

    let record = LearningRecord {
        ts: iso8601_utc(SystemTime::now()),
        repo,
        branch,
        provider: acc.provider.to_string(),
        model: acc.model.to_string(),
        format: format_name(acc.format).to_string(),
        candidates: acc.candidates.to_vec(),
        accepted_index: acc.accepted_index,
        edited_text: acc.candidates[acc.accepted_index].clone(),
        files: acc.files.to_vec(),
        diff_bytes: acc.diff.len(),
        diff: acc.diff.to_string(),
    };

    if let Err(e) = store.write(&record) {
        tracing::warn!(error = %e, "failed to record accepted commit to learning store");
        return;
    }

    // Keep the per-repo store out of version control. Only relevant when
    // the scope actually writes a repo file.
    let writes_repo = matches!(
        config.learning.scope,
        schema::LearningScope::Repo | schema::LearningScope::RepoGlobal
    );
    if config.learning.auto_gitignore
        && writes_repo
        && let Some(root) = repo_root.as_deref()
        && let Err(e) = crate::learning::ensure_gitignored(root)
    {
        tracing::warn!(error = %e, "failed to update .gitignore for the learning store");
    }
}

/// Recent accepted messages to seed the prompt (learning loop). Empty
/// when learning is disabled or the store read fails — best-effort, so
/// history problems never block generation.
fn learned_examples(config: &Config, cwd: &Path, format: schema::MessageFormat) -> Vec<String> {
    if !config.learning.enabled {
        return Vec::new();
    }
    let repo_root = git::repo_root(cwd).ok();
    let store = Store::open(config.learning.scope, repo_root.as_deref());
    store
        .load_examples(format_name(format), config.learning.max_examples as usize)
        .unwrap_or_default()
}

/// Config-file spelling of a message format, for the stored record.
fn format_name(f: schema::MessageFormat) -> &'static str {
    match f {
        schema::MessageFormat::Plain => "plain",
        schema::MessageFormat::Conventional => "conventional",
        schema::MessageFormat::ConventionalBody => "conventional+body",
        schema::MessageFormat::Gitmoji => "gitmoji",
        schema::MessageFormat::SubjectBody => "subject+body",
        schema::MessageFormat::Custom => "custom",
    }
}

/// Format a `SystemTime` as a UTC ISO-8601 / RFC-3339 instant
/// (`YYYY-MM-DDThh:mm:ssZ`). Std-only, so the learning store stays
/// clock- and dependency-free while records sort chronologically.
fn iso8601_utc(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Days-since-Unix-epoch → (year, month, day). Howard Hinnant's
/// `civil_from_days` algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
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

    #[test]
    fn iso8601_formats_known_instants() {
        assert_eq!(iso8601_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            iso8601_utc(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)),
            "2023-11-14T22:13:20Z"
        );
    }

    #[test]
    fn format_name_matches_config_spelling() {
        assert_eq!(
            format_name(schema::MessageFormat::ConventionalBody),
            "conventional+body"
        );
        assert_eq!(format_name(schema::MessageFormat::Gitmoji), "gitmoji");
        assert_eq!(format_name(schema::MessageFormat::Plain), "plain");
    }
}
