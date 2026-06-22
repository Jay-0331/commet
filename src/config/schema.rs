//! Configuration schema.
//!
//! Mirrors the `[provider]`, `[providers.*]`, `[style]`, `[learning]`,
//! `[git]`, and `[ui]` blocks from the plan's TOML schema, one Rust
//! struct per block with sensible defaults.
//!
//! This module is the **shape only**. Layered merge across defaults,
//! global, per-repo, CLI flags, and `--set` lives in
//! [`crate::config::merge`]; XDG and `git rev-parse` discovery lives in
//! [`crate::config::discover`]. Parsing is deliberately permissive:
//! missing fields fall back to defaults, unknown enum values fail with
//! a key-path error message, and unknown TOML keys are collected and
//! logged as warnings (not errors) so a forward-compatible user config
//! never locks an older binary out.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

// ---------- top-level ----------

/// Effective configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub provider: ProviderSelection,
    pub providers: Providers,
    pub style: Style,
    pub learning: Learning,
    pub git: Git,
    pub ui: Ui,
}

impl Config {
    /// Parse a TOML document into a `Config`.
    ///
    /// Emits a `tracing::warn!` for each unknown TOML key path it
    /// encounters; deserialization itself is permissive (missing
    /// fields take their default).
    pub fn from_toml_str(text: &str) -> Result<Self> {
        let value: toml::Value = toml::from_str(text).map_err(|e| Error::Config(e.to_string()))?;

        for path in find_unknown_keys(&value) {
            tracing::warn!(key = %path, "unknown config key (ignored)");
        }

        let cfg: Self = value.try_into().map_err(|e| Error::Config(e.to_string()))?;
        Ok(cfg)
    }

    /// Serialize back to TOML — used by `cc config show` once #19 lands.
    pub fn to_toml_string(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))
    }
}

// ---------- [provider] ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSelection {
    /// Which `[providers.*]` block is active.
    pub default: String,
}

impl Default for ProviderSelection {
    fn default() -> Self {
        Self {
            default: "anthropic".into(),
        }
    }
}

// ---------- [providers.*] ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Providers {
    pub anthropic: AnthropicConfig,
    pub openai: OpenAiConfig,
    pub openrouter: OpenRouterConfig,
    pub ollama: OllamaConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnthropicConfig {
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 1024,
            temperature: 0.2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiConfig {
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            model: "gpt-4o-mini".into(),
            max_tokens: 1024,
            temperature: 0.2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenRouterConfig {
    pub endpoint: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub http_referer: String,
    pub x_title: String,
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://openrouter.ai/api/v1".into(),
            model: "anthropic/claude-sonnet-4".into(),
            max_tokens: 1024,
            temperature: 0.2,
            http_referer: String::new(),
            x_title: "commitcrafter".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    pub endpoint: String,
    pub model: String,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".into(),
            model: "llama3.1:8b".into(),
        }
    }
}

// ---------- [style] ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub format: MessageFormat,
    pub subject_max_len: u32,
    pub body_wrap: u32,
    pub include_body: bool,
    pub allowed_types: Vec<String>,
    pub allowed_scopes: Vec<String>,
    pub examples: Vec<String>,
    pub generate: u32,
    pub extra_prompt: String,
    pub custom: CustomStyle,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            format: MessageFormat::Plain,
            subject_max_len: 72,
            body_wrap: 72,
            include_body: true,
            allowed_types: vec![
                "feat", "fix", "refactor", "docs", "test", "chore", "perf", "ci", "build", "style",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            allowed_scopes: Vec::new(),
            examples: vec![
                "feat(auth): add OAuth device flow".into(),
                "fix(parser): handle trailing comma in arrays".into(),
            ],
            generate: 1,
            extra_prompt: String::new(),
            custom: CustomStyle::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageFormat {
    Plain,
    Conventional,
    #[serde(rename = "conventional+body")]
    ConventionalBody,
    Gitmoji,
    #[serde(rename = "subject+body")]
    SubjectBody,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomStyle {
    pub system_prompt: String,
    pub template: String,
}

// ---------- [learning] ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Learning {
    pub enabled: bool,
    pub scope: LearningScope,
    pub max_examples: u32,
    pub store_diffs: bool,
    pub store_path: String,
}

impl Default for Learning {
    fn default() -> Self {
        Self {
            enabled: true,
            scope: LearningScope::RepoGlobal,
            max_examples: 5,
            store_diffs: false,
            store_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LearningScope {
    Off,
    Repo,
    Global,
    #[serde(rename = "repo+global")]
    RepoGlobal,
}

// ---------- [git] ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Git {
    pub auto_unstage_on_abort: bool,
    pub ignore_paths: Vec<String>,
}

impl Default for Git {
    fn default() -> Self {
        Self {
            auto_unstage_on_abort: true,
            ignore_paths: vec![
                "package-lock.json".into(),
                "*.lock".into(),
                "dist/**".into(),
            ],
        }
    }
}

// ---------- [ui] ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub theme: ThemeName,
    pub color: ColorMode,
    pub unicode: bool,
    pub custom: CustomColors,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            theme: ThemeName::Default,
            color: ColorMode::Auto,
            unicode: true,
            custom: CustomColors::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    Default,
    Mono,
    Dracula,
    #[serde(rename = "solarized-dark")]
    SolarizedDark,
    #[serde(rename = "solarized-light")]
    SolarizedLight,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomColors {
    pub fg: String,
    pub bg: String,
    pub accent: String,
    pub success: String,
    pub warning: String,
    pub error: String,
    pub muted: String,
    pub diff_add: String,
    pub diff_del: String,
    pub diff_meta: String,
    pub border: String,
}

impl Default for CustomColors {
    fn default() -> Self {
        Self {
            fg: "white".into(),
            bg: "reset".into(),
            accent: "#7aa2f7".into(),
            success: "green".into(),
            warning: "yellow".into(),
            error: "red".into(),
            muted: "bright_black".into(),
            diff_add: "green".into(),
            diff_del: "red".into(),
            diff_meta: "cyan".into(),
            border: "bright_black".into(),
        }
    }
}

// ---------- unknown-key detection ----------

/// Walk a parsed TOML document and return every key path that is not
/// part of the schema. Logs nothing; the public [`Config::from_toml_str`]
/// wraps this and emits warnings.
pub fn find_unknown_keys(value: &toml::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let toml::Value::Table(table) = value {
        walk(table, "", &mut out);
    }
    out
}

fn walk(table: &toml::Table, prefix: &str, out: &mut Vec<String>) {
    for (key, v) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        if KNOWN_KEYS.binary_search(&path.as_str()).is_ok() {
            if let toml::Value::Table(nested) = v {
                walk(nested, &path, out);
            }
        } else {
            out.push(path);
        }
    }
}

/// Every recognized dotted key path in the schema. **MUST stay sorted**
/// for `binary_search`. Drift is caught by `default_round_trips` —
/// `Config::default()` must serialize to a document whose every key is
/// in this list.
const KNOWN_KEYS: &[&str] = &[
    "git",
    "git.auto_unstage_on_abort",
    "git.ignore_paths",
    "learning",
    "learning.enabled",
    "learning.max_examples",
    "learning.scope",
    "learning.store_diffs",
    "learning.store_path",
    "provider",
    "provider.default",
    "providers",
    "providers.anthropic",
    "providers.anthropic.max_tokens",
    "providers.anthropic.model",
    "providers.anthropic.temperature",
    "providers.ollama",
    "providers.ollama.endpoint",
    "providers.ollama.model",
    "providers.openai",
    "providers.openai.max_tokens",
    "providers.openai.model",
    "providers.openai.temperature",
    "providers.openrouter",
    "providers.openrouter.endpoint",
    "providers.openrouter.http_referer",
    "providers.openrouter.max_tokens",
    "providers.openrouter.model",
    "providers.openrouter.temperature",
    "providers.openrouter.x_title",
    "style",
    "style.allowed_scopes",
    "style.allowed_types",
    "style.body_wrap",
    "style.custom",
    "style.custom.system_prompt",
    "style.custom.template",
    "style.examples",
    "style.extra_prompt",
    "style.format",
    "style.generate",
    "style.include_body",
    "style.subject_max_len",
    "ui",
    "ui.color",
    "ui.custom",
    "ui.custom.accent",
    "ui.custom.bg",
    "ui.custom.border",
    "ui.custom.diff_add",
    "ui.custom.diff_del",
    "ui.custom.diff_meta",
    "ui.custom.error",
    "ui.custom.fg",
    "ui.custom.muted",
    "ui.custom.success",
    "ui.custom.warning",
    "ui.theme",
    "ui.unicode",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_is_sorted() {
        // binary_search relies on sort order; assert it explicitly so
        // edits don't silently break unknown-key detection.
        let mut sorted = KNOWN_KEYS.to_vec();
        sorted.sort();
        assert_eq!(KNOWN_KEYS, sorted.as_slice());
    }

    #[test]
    fn default_serializes_only_known_keys() {
        // Catches drift: if a struct grows a new field, the KNOWN_KEYS
        // list must grow to match (or unknown-key detection becomes
        // useless).
        let toml_text = Config::default().to_toml_string().unwrap();
        let value: toml::Value = toml::from_str(&toml_text).unwrap();
        let unknowns = find_unknown_keys(&value);
        assert!(
            unknowns.is_empty(),
            "Config::default() produced unknown keys: {unknowns:?}",
        );
    }

    #[test]
    fn default_round_trips_through_toml() {
        let original = Config::default();
        let text = original.to_toml_string().unwrap();
        let parsed = Config::from_toml_str(&text).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn empty_input_yields_defaults() {
        let cfg = Config::from_toml_str("").unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_input_keeps_other_defaults() {
        let cfg = Config::from_toml_str(
            r#"
            [provider]
            default = "openai"

            [style]
            subject_max_len = 50
            "#,
        )
        .unwrap();

        assert_eq!(cfg.provider.default, "openai");
        assert_eq!(cfg.style.subject_max_len, 50);
        // Untouched fields stay at their default.
        assert_eq!(cfg.style.format, MessageFormat::Plain);
        assert_eq!(cfg.learning.max_examples, 5);
    }

    #[test]
    fn full_plan_example_parses() {
        let text = r#"
            [provider]
            default = "anthropic"

            [providers.anthropic]
            model = "claude-sonnet-4-6"
            max_tokens = 1024
            temperature = 0.2

            [providers.openrouter]
            endpoint = "https://openrouter.ai/api/v1"
            model = "meta-llama/llama-3.1-70b-instruct"
            max_tokens = 1024
            temperature = 0.2
            x_title = "commitcrafter"

            [style]
            format = "conventional+body"
            subject_max_len = 72
            include_body = true
            generate = 3

            [learning]
            enabled = true
            scope = "repo+global"
            max_examples = 5

            [git]
            ignore_paths = ["package-lock.json", "*.lock"]

            [ui]
            theme = "dracula"
            color = "auto"
        "#;

        let cfg = Config::from_toml_str(text).unwrap();
        assert_eq!(cfg.provider.default, "anthropic");
        assert_eq!(cfg.style.format, MessageFormat::ConventionalBody);
        assert_eq!(cfg.style.generate, 3);
        assert_eq!(
            cfg.providers.openrouter.model,
            "meta-llama/llama-3.1-70b-instruct"
        );
        assert_eq!(cfg.learning.scope, LearningScope::RepoGlobal);
        assert_eq!(cfg.ui.theme, ThemeName::Dracula);
        assert_eq!(cfg.git.ignore_paths.len(), 2);
    }

    #[test]
    fn unknown_format_value_errors_with_field_path() {
        let err = Config::from_toml_str(
            r#"
            [style]
            format = "wibble"
            "#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("style") || msg.contains("format"),
            "error should mention the key path; got: {msg}",
        );
        assert!(
            msg.contains("wibble") || msg.contains("unknown variant"),
            "error should mention the offending value; got: {msg}",
        );
    }

    #[test]
    fn unknown_scope_value_errors() {
        let err = Config::from_toml_str(
            r#"
            [learning]
            scope = "everywhere"
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("everywhere"));
    }

    #[test]
    fn unknown_theme_value_errors() {
        let err = Config::from_toml_str(
            r#"
            [ui]
            theme = "neon"
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("neon"));
    }

    #[test]
    fn unknown_keys_are_collected_not_errored() {
        let text = r#"
            [provider]
            default = "anthropic"
            flibberty = true

            [style]
            mystery = 7
            subject_max_len = 50

            [experimental]
            knob = "on"
        "#;
        let value: toml::Value = toml::from_str(text).unwrap();
        let mut paths = find_unknown_keys(&value);
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "experimental".to_string(),
                "provider.flibberty".to_string(),
                "style.mystery".to_string(),
            ],
        );

        // And the parser still succeeds.
        let cfg = Config::from_toml_str(text).unwrap();
        assert_eq!(cfg.style.subject_max_len, 50);
    }

    #[test]
    fn malformed_toml_errors() {
        let err = Config::from_toml_str("this is not = valid = toml").unwrap_err();
        assert!(matches!(err, Error::Config(_)));
    }
}
