//! Parser for `--set <dotted.path>=<value>` CLI overrides.
//!
//! Splits one argument into a `(path, toml::Value)` pair:
//!
//! ```text
//! style.subject_max_len=50           → ("style.subject_max_len", Integer(50))
//! provider.default=openai            → ("provider.default", String("openai"))
//! provider.default="openai"          → ("provider.default", String("openai"))
//! providers.openrouter.model=meta-llama/llama-3.1-70b-instruct
//!                                    → ("providers.openrouter.model",
//!                                       String("meta-llama/llama-3.1-70b-instruct"))
//! style.examples=["feat: x", "fix: y"]
//!                                    → ("style.examples", Array(...))
//! learning.enabled=false             → ("learning.enabled", Boolean(false))
//! providers.openai.temperature=0.4   → ("providers.openai.temperature", Float(0.4))
//! ```
//!
//! Value parsing strategy: wrap the right-hand side in a tiny TOML doc
//! (`v = <rhs>`) and let `toml` decide whether it's an integer, float,
//! bool, string, or array. If TOML rejects it, the value falls back to
//! a bare string — necessary because users won't quote ordinary
//! strings on the command line.
//!
//! Path validation against the schema's [`KNOWN_KEYS`] list happens at
//! [`crate::config::Layered::load`] time, not here; this module is
//! purely lexical.

use crate::error::{Error, Result};

/// Parse a single `--set <path>=<value>` argument.
pub fn parse_arg(arg: &str) -> Result<(String, toml::Value)> {
    let (path, rhs) = arg
        .split_once('=')
        .ok_or_else(|| Error::Config(format!("--set expects `key.path=value`, got `{arg}`")))?;

    let path = path.trim();
    if path.is_empty() {
        return Err(Error::Config(format!(
            "--set requires a non-empty key path, got `{arg}`",
        )));
    }
    if path.split('.').any(|seg| seg.is_empty()) {
        return Err(Error::Config(format!(
            "--set path `{path}` contains an empty segment",
        )));
    }

    let value = parse_value(rhs);
    Ok((path.to_string(), value))
}

/// Parse the right-hand side of a `--set` argument as a `toml::Value`.
///
/// First tries TOML-as-value (so `42`, `true`, `1.5`, `"quoted"`, and
/// `[1, 2]` all round-trip to their typed `Value`); on parse failure
/// falls back to a bare `String`.
fn parse_value(rhs: &str) -> toml::Value {
    let doc = format!("v = {rhs}");
    if let Ok(table) = doc.parse::<toml::Table>()
        && let Some(v) = table.get("v")
    {
        return v.clone();
    }
    toml::Value::String(rhs.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_scalar() {
        let (path, value) = parse_arg("style.subject_max_len=50").unwrap();
        assert_eq!(path, "style.subject_max_len");
        assert_eq!(value, toml::Value::Integer(50));
    }

    #[test]
    fn float_scalar() {
        let (_, value) = parse_arg("providers.openai.temperature=0.4").unwrap();
        assert_eq!(value, toml::Value::Float(0.4));
    }

    #[test]
    fn boolean_scalar() {
        let (_, value) = parse_arg("learning.enabled=false").unwrap();
        assert_eq!(value, toml::Value::Boolean(false));
    }

    #[test]
    fn quoted_string() {
        let (_, value) = parse_arg(r#"provider.default="openai""#).unwrap();
        assert_eq!(value, toml::Value::String("openai".into()));
    }

    #[test]
    fn bare_string_with_unquoted_word() {
        let (_, value) = parse_arg("provider.default=openai").unwrap();
        assert_eq!(value, toml::Value::String("openai".into()));
    }

    #[test]
    fn bare_string_with_punctuation_and_slashes() {
        let (path, value) =
            parse_arg("providers.openrouter.model=meta-llama/llama-3.1-70b-instruct").unwrap();
        assert_eq!(path, "providers.openrouter.model");
        assert_eq!(
            value,
            toml::Value::String("meta-llama/llama-3.1-70b-instruct".into()),
        );
    }

    #[test]
    fn array_value() {
        let (_, value) = parse_arg(r#"style.examples=["feat: x", "fix: y"]"#).unwrap();
        let arr = value.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_str(), Some("feat: x"));
    }

    #[test]
    fn empty_rhs_becomes_empty_string() {
        let (path, value) = parse_arg("provider.default=").unwrap();
        assert_eq!(path, "provider.default");
        assert_eq!(value, toml::Value::String(String::new()));
    }

    #[test]
    fn extra_equals_in_value_kept_intact() {
        // Split on the first `=` only; everything after stays as-is.
        let (path, value) = parse_arg("style.extra_prompt=foo=bar=baz").unwrap();
        assert_eq!(path, "style.extra_prompt");
        assert_eq!(value, toml::Value::String("foo=bar=baz".into()));
    }

    #[test]
    fn whitespace_around_path_trimmed() {
        let (path, _) = parse_arg("  style.format  =gitmoji").unwrap();
        assert_eq!(path, "style.format");
    }

    #[test]
    fn missing_equals_errors() {
        let err = parse_arg("style.format").unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("style.format")));
    }

    #[test]
    fn empty_path_errors() {
        let err = parse_arg("=value").unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("non-empty key path")));
    }

    #[test]
    fn empty_segment_errors() {
        let err = parse_arg("style..format=plain").unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("empty segment")));
    }
}
