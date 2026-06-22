//! Layered deep merge of configuration with per-leaf source tracking.
//!
//! The merge always starts from [`Config::default`] and lays each
//! optional layer on top in this order (low → high):
//!
//! 1. defaults
//! 2. global file
//! 3. per-repo file
//! 4. CLI flags (constructed by the caller)
//! 5. `--set <key.path>=<value>` overrides
//!
//! Within a layer we deep-merge tables — descending into matching
//! sub-tables and only replacing leaves — while arrays and scalars
//! are wholesale replaced (per the plan: predictability beats
//! cleverness). Each leaf written records the [`Source`] in a parallel
//! [`Sources`] map so `cc config show` can answer "where did this
//! value come from?" without re-reading any file.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

use super::schema::{Config, KNOWN_KEYS, find_unknown_keys};
use super::set_override;
use super::source::{Source, Sources};

/// Builder for a layered config load. See the module-level docs for
/// the precedence rules.
#[derive(Debug, Default)]
pub struct Layered {
    global: Option<(PathBuf, toml::Value)>,
    repo: Option<(PathBuf, toml::Value)>,
    flag: Option<toml::Value>,
    sets: Vec<(String, toml::Value)>,
}

/// Result of [`Layered::load`].
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub config: Config,
    pub sources: Sources,
}

impl Layered {
    /// Start with no layers (just defaults).
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the global config file from disk and store it as the
    /// global layer. Existing global state is replaced.
    pub fn with_global_file(self, path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref().to_path_buf();
        let value = parse_file(&p)?;
        Ok(self.with_global_value(p, value))
    }

    /// Store a pre-parsed `toml::Value` as the global layer (used by
    /// tests so they don't have to write a real file).
    pub fn with_global_value(mut self, path: PathBuf, value: toml::Value) -> Self {
        self.global = Some((path, value));
        self
    }

    /// Read the per-repo config file from disk and store it as the
    /// repo layer.
    pub fn with_repo_file(self, path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref().to_path_buf();
        let value = parse_file(&p)?;
        Ok(self.with_repo_value(p, value))
    }

    /// Pre-parsed variant of [`Self::with_repo_file`].
    pub fn with_repo_value(mut self, path: PathBuf, value: toml::Value) -> Self {
        self.repo = Some((path, value));
        self
    }

    /// Replace the entire CLI-flag layer. Callers build this from
    /// parsed clap args; intentionally not coupled to the `cli` module
    /// so tests can construct minimal flag layers by hand.
    pub fn with_flag_value(mut self, value: toml::Value) -> Self {
        self.flag = Some(value);
        self
    }

    /// Append a `--set <path>=<value>` override. Repeatable; later
    /// pushes overwrite earlier ones at the same path because they
    /// are applied in insertion order.
    pub fn with_set(mut self, path: impl Into<String>, value: toml::Value) -> Self {
        self.sets.push((path.into(), value));
        self
    }

    /// Parse one `--set` CLI argument (`key.path=value`) and push it
    /// onto the set layer. Returns the same builder for chaining.
    pub fn with_set_arg(mut self, arg: &str) -> Result<Self> {
        let (path, value) = set_override::parse_arg(arg)?;
        self.sets.push((path, value));
        Ok(self)
    }

    /// Perform the merge and deserialize into a typed [`Config`].
    pub fn load(self) -> Result<Loaded> {
        // Defaults: serialize Config::default() to TOML and seed
        // sources with Source::Default for every leaf.
        let default_cfg = Config::default();
        let mut acc: toml::Value = toml::Value::try_from(&default_cfg)
            .map_err(|e| Error::Config(format!("serialize defaults: {e}")))?;
        let mut sources = Sources::new();
        record_leaves(&acc, &Source::Default, &mut sources, "");

        // Warn about unknown keys in each user-provided file layer.
        // (The schema module already does this for the standalone
        // `Config::from_toml_str` path; do it again here so layered
        // loads surface them too.)
        if let Some((path, value)) = &self.global {
            for unknown in find_unknown_keys(value) {
                tracing::warn!(
                    file = %path.display(),
                    key = %unknown,
                    "unknown config key (ignored)",
                );
            }
        }
        if let Some((path, value)) = &self.repo {
            for unknown in find_unknown_keys(value) {
                tracing::warn!(
                    file = %path.display(),
                    key = %unknown,
                    "unknown config key (ignored)",
                );
            }
        }

        if let Some((path, value)) = self.global {
            merge_into(&mut acc, &value, &Source::Global(path), &mut sources, "");
        }
        if let Some((path, value)) = self.repo {
            merge_into(&mut acc, &value, &Source::Repo(path), &mut sources, "");
        }
        if let Some(value) = self.flag {
            merge_into(&mut acc, &value, &Source::Flag, &mut sources, "");
        }
        // Validate every --set path: syntactic shape first (empty
        // path, empty segments), then membership in `KNOWN_KEYS`.
        // Catches typos like `style.subjct_max_len=50` and points at
        // the closest legitimate key.
        for (path, _) in &self.sets {
            validate_set_path_syntax(path)?;
            if !KNOWN_KEYS.contains(&path.as_str()) {
                return Err(Error::Config(format!(
                    "unknown --set path `{path}`{}",
                    close_match_hint(path),
                )));
            }
        }

        for (path, value) in self.sets {
            apply_set(&mut acc, &path, value, &mut sources)?;
        }

        let config: Config = acc.try_into().map_err(|e| Error::Config(e.to_string()))?;
        Ok(Loaded { config, sources })
    }
}

/// Build a "did you mean: ..." hint for an unknown --set path.
///
/// Returns either an empty string (no close match) or `\n  did you
/// mean: a, b, c?` listing up to three closest schema keys.
fn close_match_hint(unknown: &str) -> String {
    const MAX_DISTANCE: usize = 4;
    const MAX_SUGGESTIONS: usize = 3;

    let mut scored: Vec<(usize, &&str)> = KNOWN_KEYS
        .iter()
        .map(|k| (levenshtein(k, unknown), k))
        .filter(|(d, _)| *d <= MAX_DISTANCE)
        .collect();
    if scored.is_empty() {
        return String::new();
    }
    scored.sort_by_key(|(d, _)| *d);

    let suggestions: Vec<&str> = scored
        .iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, k)| **k)
        .collect();

    format!("\n  did you mean: {}?", suggestions.join(", "))
}

/// Iterative Levenshtein distance — small, allocator-light, no deps.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr: Vec<usize> = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Read a TOML file from disk and parse it.
fn parse_file(path: &Path) -> Result<toml::Value> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("read {}: {e}", path.display())))?;
    toml::from_str(&text).map_err(|e| Error::Config(format!("parse {}: {e}", path.display())))
}

/// Walk a TOML tree and record every leaf path against the given
/// source. Tables recurse; arrays and scalars are leaves.
fn record_leaves(value: &toml::Value, source: &Source, sources: &mut Sources, prefix: &str) {
    match value {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let path = join_path(prefix, k);
                record_leaves(v, source, sources, &path);
            }
        }
        _ => sources.set(prefix.to_string(), source.clone()),
    }
}

/// Deep-merge `layer` into `acc`. Tables recurse, everything else
/// (including arrays) is a full replacement. Sources for each
/// touched leaf are updated to the given source.
fn merge_into(
    acc: &mut toml::Value,
    layer: &toml::Value,
    source: &Source,
    sources: &mut Sources,
    prefix: &str,
) {
    // Fast path: when both sides are tables, descend without
    // touching the outer value.
    if let (toml::Value::Table(a), toml::Value::Table(b)) = (&mut *acc, layer) {
        for (k, layer_v) in b {
            let child_path = join_path(prefix, k);
            match a.get_mut(k) {
                Some(acc_v) => {
                    // Both present.
                    if acc_v.is_table() && layer_v.is_table() {
                        merge_into(acc_v, layer_v, source, sources, &child_path);
                    } else {
                        *acc_v = layer_v.clone();
                        if layer_v.is_table() {
                            // Promoted from non-table to table: attribute
                            // every new leaf to this source.
                            record_leaves(layer_v, source, sources, &child_path);
                        } else {
                            sources.set(child_path.clone(), source.clone());
                        }
                    }
                }
                None => {
                    // New key — clone in and record all leaves.
                    a.insert(k.clone(), layer_v.clone());
                    record_leaves(layer_v, source, sources, &child_path);
                }
            }
        }
        return;
    }

    // Non-table vs anything: replace wholesale.
    *acc = layer.clone();
    if layer.is_table() {
        record_leaves(layer, source, sources, prefix);
    } else {
        sources.set(prefix.to_string(), source.clone());
    }
}

/// Syntactic check for a `--set` path (no schema knowledge).
fn validate_set_path_syntax(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(Error::Config("empty --set path".into()));
    }
    if path.split('.').any(|p| p.is_empty()) {
        return Err(Error::Config(format!(
            "invalid --set path `{path}` (empty segment)"
        )));
    }
    Ok(())
}

/// Apply a `--set <dotted.path>=<value>` override.
///
/// Caller must have already validated `path` via
/// [`validate_set_path_syntax`] and confirmed it is in `KNOWN_KEYS`.
fn apply_set(
    acc: &mut toml::Value,
    path: &str,
    value: toml::Value,
    sources: &mut Sources,
) -> Result<()> {
    let parts: Vec<&str> = path.split('.').collect();

    // Walk to the parent of the final segment, creating tables as we go.
    let mut current: &mut toml::Value = acc;
    for part in &parts[..parts.len() - 1] {
        if !current.is_table() {
            return Err(Error::Config(format!(
                "cannot descend into non-table while applying --set `{path}`"
            )));
        }
        let tbl = current.as_table_mut().unwrap();
        current = tbl
            .entry((*part).to_string())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    }

    let tbl = current
        .as_table_mut()
        .ok_or_else(|| Error::Config(format!("parent of --set `{path}` is not a table")))?;
    let last = *parts
        .last()
        .expect("non-empty after validate_set_path_syntax");
    tbl.insert(last.to_string(), value.clone());

    // Source tracking: the leaf itself is always `Source::Set`. If the
    // value is a sub-table, every leaf inside it also becomes `Source::Set`.
    if value.is_table() {
        record_leaves(&value, &Source::Set, sources, path);
    } else {
        sources.set(path.to_string(), Source::Set);
    }
    Ok(())
}

fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{LearningScope, MessageFormat, ThemeName};

    fn anthropic_value(model: &str) -> toml::Value {
        toml::from_str(&format!(
            r#"
            [providers.anthropic]
            model = "{model}"
            "#
        ))
        .unwrap()
    }

    #[test]
    fn defaults_only_loads_baked_in_values() {
        let loaded = Layered::new().load().unwrap();
        assert_eq!(loaded.config, Config::default());

        // Every leaf in Config::default() must be tracked as Source::Default.
        let known = loaded.sources.len();
        assert!(known > 0, "sources map should be populated");
        for (path, src) in loaded.sources.iter() {
            assert_eq!(
                src,
                &Source::Default,
                "{path} should be Source::Default but was {src:?}",
            );
        }
    }

    #[test]
    fn global_overrides_defaults_and_sources_track_global() {
        let global_path = PathBuf::from("/cfg/global.toml");
        let loaded = Layered::new()
            .with_global_value(global_path.clone(), anthropic_value("opus-fake"))
            .load()
            .unwrap();

        assert_eq!(loaded.config.providers.anthropic.model, "opus-fake");
        // Untouched leaves remain Default.
        assert_eq!(
            loaded.sources.get("providers.openai.model"),
            Some(&Source::Default),
        );
        // The overridden leaf is Global.
        assert_eq!(
            loaded.sources.get("providers.anthropic.model"),
            Some(&Source::Global(global_path)),
        );
    }

    #[test]
    fn repo_overrides_global() {
        let global = anthropic_value("global-model");
        let repo = anthropic_value("repo-model");
        let loaded = Layered::new()
            .with_global_value(PathBuf::from("/g.toml"), global)
            .with_repo_value(PathBuf::from("/r.toml"), repo)
            .load()
            .unwrap();

        assert_eq!(loaded.config.providers.anthropic.model, "repo-model");
        assert_eq!(
            loaded.sources.get("providers.anthropic.model"),
            Some(&Source::Repo(PathBuf::from("/r.toml"))),
        );
    }

    #[test]
    fn flag_overrides_repo() {
        let loaded = Layered::new()
            .with_repo_value(PathBuf::from("/r.toml"), anthropic_value("repo-model"))
            .with_flag_value(anthropic_value("flag-model"))
            .load()
            .unwrap();
        assert_eq!(loaded.config.providers.anthropic.model, "flag-model");
        assert_eq!(
            loaded.sources.get("providers.anthropic.model"),
            Some(&Source::Flag),
        );
    }

    #[test]
    fn set_overrides_flag() {
        let loaded = Layered::new()
            .with_flag_value(anthropic_value("flag-model"))
            .with_set(
                "providers.anthropic.model",
                toml::Value::String("set-model".into()),
            )
            .load()
            .unwrap();
        assert_eq!(loaded.config.providers.anthropic.model, "set-model");
        assert_eq!(
            loaded.sources.get("providers.anthropic.model"),
            Some(&Source::Set),
        );
    }

    #[test]
    fn arrays_are_replaced_not_concatenated() {
        let global: toml::Value = toml::from_str(
            r#"
            [git]
            ignore_paths = ["*.foo"]
            "#,
        )
        .unwrap();
        let repo: toml::Value = toml::from_str(
            r#"
            [git]
            ignore_paths = ["*.bar", "*.baz"]
            "#,
        )
        .unwrap();
        let loaded = Layered::new()
            .with_global_value(PathBuf::from("/g.toml"), global)
            .with_repo_value(PathBuf::from("/r.toml"), repo)
            .load()
            .unwrap();
        assert_eq!(loaded.config.git.ignore_paths, vec!["*.bar", "*.baz"]);
    }

    #[test]
    fn partial_layer_leaves_other_keys_default() {
        let global: toml::Value = toml::from_str(
            r#"
            [provider]
            default = "openai"
            "#,
        )
        .unwrap();
        let loaded = Layered::new()
            .with_global_value(PathBuf::from("/g.toml"), global)
            .load()
            .unwrap();

        assert_eq!(loaded.config.provider.default, "openai");
        // Defaults survive.
        assert_eq!(loaded.config.style.subject_max_len, 72);
        assert_eq!(loaded.config.learning.max_examples, 5);
        // Sources reflect this.
        assert_eq!(
            loaded.sources.get("provider.default"),
            Some(&Source::Global(PathBuf::from("/g.toml"))),
        );
        assert_eq!(
            loaded.sources.get("style.subject_max_len"),
            Some(&Source::Default),
        );
    }

    #[test]
    fn set_into_deeply_nested_known_path() {
        let loaded = Layered::new()
            .with_set(
                "providers.openrouter.x_title",
                toml::Value::String("my-tool".into()),
            )
            .load()
            .unwrap();
        assert_eq!(loaded.config.providers.openrouter.x_title, "my-tool");
        assert_eq!(
            loaded.sources.get("providers.openrouter.x_title"),
            Some(&Source::Set),
        );
    }

    #[test]
    fn unknown_set_path_errors_with_close_match_suggestion() {
        // `subjct_max_len` is a one-character typo of `subject_max_len`.
        let err = Layered::new()
            .with_set("style.subjct_max_len", toml::Value::Integer(50))
            .load()
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown --set path"),
            "expected unknown-path error, got: {msg}",
        );
        assert!(
            msg.contains("style.subject_max_len"),
            "expected close-match suggestion, got: {msg}",
        );
    }

    #[test]
    fn unknown_set_path_with_no_close_match_omits_suggestion_block() {
        let err = Layered::new()
            .with_set("xyz123.totally.wrong", toml::Value::Integer(1))
            .load()
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown --set path"));
        // No suggestion line — distance to any KNOWN_KEY is greater than the threshold.
        assert!(
            !msg.contains("did you mean"),
            "did-you-mean shouldn't fire for distant strings, got: {msg}",
        );
    }

    #[test]
    fn with_set_arg_parses_and_pushes() {
        let loaded = Layered::new()
            .with_set_arg("style.subject_max_len=80")
            .unwrap()
            .load()
            .unwrap();
        assert_eq!(loaded.config.style.subject_max_len, 80);
        assert_eq!(
            loaded.sources.get("style.subject_max_len"),
            Some(&Source::Set),
        );
    }

    #[test]
    fn with_set_arg_supports_bare_string_value() {
        let loaded = Layered::new()
            .with_set_arg("providers.openrouter.model=meta-llama/llama-3.1-70b-instruct")
            .unwrap()
            .load()
            .unwrap();
        assert_eq!(
            loaded.config.providers.openrouter.model,
            "meta-llama/llama-3.1-70b-instruct",
        );
    }

    #[test]
    fn with_set_arg_propagates_parse_errors() {
        let err = Layered::new().with_set_arg("no-equals-here").unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("key.path=value")));
    }

    #[test]
    fn levenshtein_basic_cases() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        // One insert (the `e` in `subject`) — distance 1.
        assert_eq!(
            levenshtein("style.subjct_max_len", "style.subject_max_len"),
            1,
        );
    }

    #[test]
    fn set_handles_repeated_paths_with_last_winning() {
        let loaded = Layered::new()
            .with_set("provider.default", toml::Value::String("openai".into()))
            .with_set("provider.default", toml::Value::String("ollama".into()))
            .load()
            .unwrap();
        assert_eq!(loaded.config.provider.default, "ollama");
    }

    #[test]
    fn set_rejects_empty_path() {
        let err = Layered::new()
            .with_set("", toml::Value::Boolean(true))
            .load()
            .unwrap_err();
        assert!(matches!(err, Error::Config(_)));
    }

    #[test]
    fn set_rejects_path_with_empty_segment() {
        let err = Layered::new()
            .with_set("provider..default", toml::Value::String("x".into()))
            .load()
            .unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("empty segment")));
    }

    #[test]
    fn invalid_enum_value_via_set_fails_load() {
        let err = Layered::new()
            .with_set("style.format", toml::Value::String("wibble".into()))
            .load()
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("wibble") || msg.contains("unknown variant"),
            "expected enum validation error, got: {msg}",
        );
    }

    #[test]
    fn set_into_typed_enum_field_works() {
        let loaded = Layered::new()
            .with_set("style.format", toml::Value::String("gitmoji".into()))
            .load()
            .unwrap();
        assert_eq!(loaded.config.style.format, MessageFormat::Gitmoji);
        assert_eq!(loaded.sources.get("style.format"), Some(&Source::Set));
    }

    #[test]
    fn set_into_learning_scope_works() {
        let loaded = Layered::new()
            .with_set("learning.scope", toml::Value::String("off".into()))
            .load()
            .unwrap();
        assert_eq!(loaded.config.learning.scope, LearningScope::Off);
    }

    #[test]
    fn flag_can_change_ui_color_and_theme() {
        let flag: toml::Value = toml::from_str(
            r#"
            [ui]
            color = "never"
            theme = "dracula"
            "#,
        )
        .unwrap();
        let loaded = Layered::new().with_flag_value(flag).load().unwrap();
        assert_eq!(
            loaded.config.ui.color,
            super::super::schema::ColorMode::Never
        );
        assert_eq!(loaded.config.ui.theme, ThemeName::Dracula);
        assert_eq!(loaded.sources.get("ui.color"), Some(&Source::Flag));
    }

    #[test]
    fn join_path_helper() {
        assert_eq!(join_path("", "a"), "a");
        assert_eq!(join_path("a", "b"), "a.b");
        assert_eq!(join_path("a.b", "c"), "a.b.c");
    }

    #[test]
    fn malformed_toml_in_file_layer_surfaces_path() {
        // Write a temp file with broken TOML; with_global_file should
        // surface the path in the error message.
        let tmp = std::env::temp_dir().join(format!(
            "commitcrafter-merge-bad-{}.toml",
            std::process::id()
        ));
        std::fs::write(&tmp, "this is = not = toml").unwrap();
        let err = Layered::new().with_global_file(&tmp).unwrap_err();
        let msg = err.to_string();
        let _ = std::fs::remove_file(&tmp);
        assert!(
            msg.contains(tmp.file_name().unwrap().to_str().unwrap())
                || msg.contains(tmp.to_string_lossy().as_ref()),
            "error should mention the failing file path; got: {msg}",
        );
    }

    #[test]
    fn missing_file_surfaces_path_in_error() {
        let tmp = PathBuf::from("/nonexistent/commitcrafter-config-does-not-exist.toml");
        let err = Layered::new().with_global_file(&tmp).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("nonexistent") || msg.contains(tmp.to_string_lossy().as_ref()),
            "missing-file error should mention the path; got: {msg}",
        );
    }
}
