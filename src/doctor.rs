//! Preflight checks shared by `commet doctor` and `commet setup`.
//!
//! Each check is a pure function of a [`CheckCtx`] — the environment is
//! probed once into the context (by the command, #51), then the checks
//! just interpret it. That keeps every check unit-testable against a
//! stubbed context and lets the two commands run the identical list.
//!
//! Checks are **synchronous**: the whole crate uses a blocking HTTP
//! client, so there's no runtime to make `run` async against (a
//! deliberate simplification of the issue's `async fn` sketch).

use crate::tui::ColorCap;

/// Outcome of a single check. Every variant carries a short message; a
/// `fix_hint` is attached to actionable `Warn`/`Fail` results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Ok(String),
    Warn(String),
    Fail(String),
}

/// A named check outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub name: &'static str,
    pub status: Status,
    pub fix_hint: Option<String>,
}

impl CheckResult {
    fn ok(name: &'static str, msg: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Ok(msg.into()),
            fix_hint: None,
        }
    }
    fn warn(name: &'static str, msg: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Warn(msg.into()),
            fix_hint: Some(hint.into()),
        }
    }
    fn fail(name: &'static str, msg: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Fail(msg.into()),
            fix_hint: Some(hint.into()),
        }
    }

    /// Whether this result is a hard failure.
    pub fn is_fail(&self) -> bool {
        matches!(self.status, Status::Fail(_))
    }
}

/// Environment snapshot the checks interpret. The command probes the
/// real environment into this; tests build it by hand.
#[derive(Debug, Clone)]
pub struct CheckCtx {
    /// `git --version` output, or `None` when git isn't on `PATH`.
    pub git_version: Option<String>,
    /// Whether the cwd is inside a git working tree.
    pub in_repo: bool,
    /// Config load error (global/repo), if any.
    pub config_error: Option<String>,
    /// Per-repo config parse error, if any.
    pub repo_config_error: Option<String>,
    /// The configured default provider name.
    pub provider: String,
    /// Whether that provider name is a known builtin.
    pub provider_registered: bool,
    /// The provider's API-key env var, or `None` if it needs no key.
    pub key_env: Option<String>,
    /// Whether that env var is set (meaningless when `key_env` is None).
    pub key_present: bool,
    /// Reachability probe result; `None` when not attempted.
    pub reachable: Option<bool>,
    /// `$EDITOR`/`$VISUAL`, if set.
    pub editor: Option<String>,
    /// Detected terminal color capability.
    pub color: ColorCap,
    /// Whether a clipboard backend is available.
    pub clipboard_available: bool,
    /// Whether the learning store directory is writable.
    pub store_writable: bool,
}

/// Run every check in a stable order.
pub fn run_all(ctx: &CheckCtx) -> Vec<CheckResult> {
    vec![
        git_available(ctx),
        in_repo(ctx),
        config_readable(ctx),
        repo_config_valid(ctx),
        provider_registered(ctx),
        provider_key(ctx),
        provider_reachable(ctx),
        editor_configured(ctx),
        color_support(ctx),
        clipboard_backend(ctx),
        learning_store_writable(ctx),
    ]
}

fn git_available(ctx: &CheckCtx) -> CheckResult {
    match &ctx.git_version {
        Some(v) => CheckResult::ok("git available", v.clone()),
        None => CheckResult::fail(
            "git available",
            "git not found on PATH",
            "install git and ensure it's on your PATH",
        ),
    }
}

fn in_repo(ctx: &CheckCtx) -> CheckResult {
    if ctx.in_repo {
        CheckResult::ok("inside a git repo", "yes")
    } else {
        CheckResult::warn(
            "inside a git repo",
            "not inside a git working tree",
            "run commet from inside a repository (`git init` if needed)",
        )
    }
}

fn config_readable(ctx: &CheckCtx) -> CheckResult {
    match &ctx.config_error {
        None => CheckResult::ok("config readable", "loaded"),
        Some(e) => CheckResult::fail(
            "config readable",
            e.clone(),
            "fix or remove the offending config file",
        ),
    }
}

fn repo_config_valid(ctx: &CheckCtx) -> CheckResult {
    match &ctx.repo_config_error {
        None => CheckResult::ok("per-repo config valid", "ok"),
        Some(e) => CheckResult::fail(
            "per-repo config valid",
            e.clone(),
            "fix `.commet.toml` at the repo root",
        ),
    }
}

fn provider_registered(ctx: &CheckCtx) -> CheckResult {
    if ctx.provider_registered {
        CheckResult::ok("provider registered", ctx.provider.clone())
    } else {
        CheckResult::fail(
            "provider registered",
            format!("unknown provider `{}`", ctx.provider),
            "set [provider].default to anthropic, openai, openrouter, or ollama",
        )
    }
}

fn provider_key(ctx: &CheckCtx) -> CheckResult {
    match &ctx.key_env {
        None => CheckResult::ok("provider API key", "no key required"),
        Some(var) if ctx.key_present => CheckResult::ok("provider API key", format!("${var} set")),
        Some(var) => CheckResult::fail(
            "provider API key",
            format!("${var} is not set"),
            format!("export {var}=<your key>"),
        ),
    }
}

fn provider_reachable(ctx: &CheckCtx) -> CheckResult {
    match ctx.reachable {
        Some(true) => CheckResult::ok("provider reachable", "responded"),
        Some(false) => CheckResult::warn(
            "provider reachable",
            "provider endpoint did not respond",
            "check your network and the provider endpoint/URL",
        ),
        None => CheckResult::ok("provider reachable", "not checked"),
    }
}

fn editor_configured(ctx: &CheckCtx) -> CheckResult {
    match &ctx.editor {
        Some(e) => CheckResult::ok("editor configured", e.clone()),
        None => CheckResult::warn(
            "editor configured",
            "$EDITOR/$VISUAL not set",
            "export EDITOR=<your editor> to use the preview's edit action",
        ),
    }
}

fn color_support(ctx: &CheckCtx) -> CheckResult {
    let label = match ctx.color {
        ColorCap::TrueColor => "truecolor",
        ColorCap::Ansi256 => "256-color",
        ColorCap::Ansi16 => "16-color",
        ColorCap::None => "no color",
    };
    CheckResult::ok("color support", label)
}

fn clipboard_backend(ctx: &CheckCtx) -> CheckResult {
    if ctx.clipboard_available {
        CheckResult::ok("clipboard backend", "available")
    } else {
        CheckResult::warn(
            "clipboard backend",
            "no clipboard backend available",
            "the -c/--clipboard flag won't work here",
        )
    }
}

fn learning_store_writable(ctx: &CheckCtx) -> CheckResult {
    if ctx.store_writable {
        CheckResult::ok("learning store writable", "ok")
    } else {
        CheckResult::warn(
            "learning store writable",
            "learning store directory is not writable",
            "check permissions on the store directory",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fully-healthy context; tests mutate one field at a time.
    fn healthy() -> CheckCtx {
        CheckCtx {
            git_version: Some("git version 2.44.0".into()),
            in_repo: true,
            config_error: None,
            repo_config_error: None,
            provider: "anthropic".into(),
            provider_registered: true,
            key_env: Some("ANTHROPIC_API_KEY".into()),
            key_present: true,
            reachable: Some(true),
            editor: Some("vim".into()),
            color: ColorCap::TrueColor,
            clipboard_available: true,
            store_writable: true,
        }
    }

    fn status_of<'a>(results: &'a [CheckResult], name: &str) -> &'a Status {
        &results.iter().find(|r| r.name == name).unwrap().status
    }

    #[test]
    fn all_healthy_yields_all_ok() {
        let results = run_all(&healthy());
        assert_eq!(results.len(), 11);
        assert!(results.iter().all(|r| matches!(r.status, Status::Ok(_))));
        assert!(!results.iter().any(|r| r.is_fail()));
    }

    #[test]
    fn missing_git_fails() {
        let mut ctx = healthy();
        ctx.git_version = None;
        let r = run_all(&ctx);
        assert!(matches!(status_of(&r, "git available"), Status::Fail(_)));
    }

    #[test]
    fn not_in_repo_warns() {
        let mut ctx = healthy();
        ctx.in_repo = false;
        assert!(matches!(
            status_of(&run_all(&ctx), "inside a git repo"),
            Status::Warn(_)
        ));
    }

    #[test]
    fn config_and_repo_config_errors_fail() {
        let mut ctx = healthy();
        ctx.config_error = Some("bad toml".into());
        ctx.repo_config_error = Some("bad repo toml".into());
        let r = run_all(&ctx);
        assert!(matches!(status_of(&r, "config readable"), Status::Fail(_)));
        assert!(matches!(
            status_of(&r, "per-repo config valid"),
            Status::Fail(_)
        ));
    }

    #[test]
    fn unknown_provider_fails() {
        let mut ctx = healthy();
        ctx.provider = "bogus".into();
        ctx.provider_registered = false;
        assert!(matches!(
            status_of(&run_all(&ctx), "provider registered"),
            Status::Fail(_)
        ));
    }

    #[test]
    fn missing_key_fails_with_var_in_hint() {
        let mut ctx = healthy();
        ctx.key_present = false;
        let r = run_all(&ctx);
        let key = r.iter().find(|r| r.name == "provider API key").unwrap();
        assert!(matches!(key.status, Status::Fail(_)));
        assert!(
            key.fix_hint
                .as_deref()
                .unwrap()
                .contains("ANTHROPIC_API_KEY")
        );
    }

    #[test]
    fn no_key_required_is_ok() {
        let mut ctx = healthy();
        ctx.key_env = None; // e.g. ollama
        ctx.key_present = false;
        assert!(matches!(
            status_of(&run_all(&ctx), "provider API key"),
            Status::Ok(_)
        ));
    }

    #[test]
    fn unreachable_warns_and_unprobed_is_ok() {
        let mut ctx = healthy();
        ctx.reachable = Some(false);
        assert!(matches!(
            status_of(&run_all(&ctx), "provider reachable"),
            Status::Warn(_)
        ));
        ctx.reachable = None;
        assert!(matches!(
            status_of(&run_all(&ctx), "provider reachable"),
            Status::Ok(_)
        ));
    }

    #[test]
    fn missing_editor_and_clipboard_and_store_warn() {
        let mut ctx = healthy();
        ctx.editor = None;
        ctx.clipboard_available = false;
        ctx.store_writable = false;
        let r = run_all(&ctx);
        assert!(matches!(
            status_of(&r, "editor configured"),
            Status::Warn(_)
        ));
        assert!(matches!(
            status_of(&r, "clipboard backend"),
            Status::Warn(_)
        ));
        assert!(matches!(
            status_of(&r, "learning store writable"),
            Status::Warn(_)
        ));
    }

    #[test]
    fn color_support_reports_level() {
        let mut ctx = healthy();
        ctx.color = ColorCap::None;
        assert_eq!(
            status_of(&run_all(&ctx), "color support"),
            &Status::Ok("no color".into())
        );
    }
}
