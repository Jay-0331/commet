//! End-to-end tests for layered config loading.
//!
//! Unit tests in `src/config/merge.rs` exercise the merge algorithm
//! with in-memory `toml::Value`s. The cases here drive the full
//! `Layered::with_global_file` / `with_repo_file` path with real
//! files in a tempdir.

use std::fs;

use commitcrafter::config::{Layered, Source};

fn write(path: &std::path::Path, text: &str) {
    fs::write(path, text).expect("write fixture");
}

#[test]
fn global_file_overrides_defaults_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global.toml");
    write(
        &global,
        r#"
        [provider]
        default = "openai"

        [providers.openai]
        model = "gpt-4o"
        temperature = 0.7
        "#,
    );

    let loaded = Layered::new()
        .with_global_file(&global)
        .unwrap()
        .load()
        .unwrap();

    assert_eq!(loaded.config.provider.default, "openai");
    assert_eq!(loaded.config.providers.openai.model, "gpt-4o");
    // Untouched defaults survive.
    assert_eq!(loaded.config.providers.openai.max_tokens, 1024);
    assert_eq!(loaded.config.style.subject_max_len, 72);

    // Sources reflect which file won.
    assert_eq!(
        loaded.sources.get("provider.default"),
        Some(&Source::Global(global.clone())),
    );
    assert_eq!(
        loaded.sources.get("providers.openai.temperature"),
        Some(&Source::Global(global)),
    );
    assert_eq!(
        loaded.sources.get("style.subject_max_len"),
        Some(&Source::Default),
    );
}

#[test]
fn repo_file_overrides_global_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global.toml");
    let repo = tmp.path().join(".commitcrafter.toml");
    write(
        &global,
        r#"
        [providers.anthropic]
        model = "claude-3-opus"
        max_tokens = 2048
        "#,
    );
    write(
        &repo,
        r#"
        [providers.anthropic]
        model = "claude-sonnet-from-repo"
        "#,
    );

    let loaded = Layered::new()
        .with_global_file(&global)
        .unwrap()
        .with_repo_file(&repo)
        .unwrap()
        .load()
        .unwrap();

    // Repo wins for model.
    assert_eq!(
        loaded.config.providers.anthropic.model,
        "claude-sonnet-from-repo",
    );
    // Global wins where repo is silent.
    assert_eq!(loaded.config.providers.anthropic.max_tokens, 2048);

    assert_eq!(
        loaded.sources.get("providers.anthropic.model"),
        Some(&Source::Repo(repo)),
    );
    assert_eq!(
        loaded.sources.get("providers.anthropic.max_tokens"),
        Some(&Source::Global(global)),
    );
}

#[test]
fn full_precedence_chain_default_global_repo_flag_set() {
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global.toml");
    let repo = tmp.path().join(".commitcrafter.toml");
    write(
        &global,
        r#"
        [providers.anthropic]
        model = "from-global"
        "#,
    );
    write(
        &repo,
        r#"
        [providers.anthropic]
        model = "from-repo"
        "#,
    );

    let flag_value: toml::Value = toml::from_str(
        r#"
        [providers.anthropic]
        model = "from-flag"
        "#,
    )
    .unwrap();

    let loaded = Layered::new()
        .with_global_file(&global)
        .unwrap()
        .with_repo_file(&repo)
        .unwrap()
        .with_flag_value(flag_value)
        .with_set(
            "providers.anthropic.model",
            toml::Value::String("from-set".into()),
        )
        .load()
        .unwrap();

    assert_eq!(loaded.config.providers.anthropic.model, "from-set");
    assert_eq!(
        loaded.sources.get("providers.anthropic.model"),
        Some(&Source::Set),
    );
}

#[test]
fn unknown_keys_in_files_do_not_error() {
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global.toml");
    write(
        &global,
        r#"
        [provider]
        default = "anthropic"
        mystery = "value"

        [experimental]
        knob = "on"

        [providers.anthropic]
        model = "claude-sonnet-4-6"
        "#,
    );

    let loaded = Layered::new()
        .with_global_file(&global)
        .unwrap()
        .load()
        .unwrap();

    // Known keys still applied.
    assert_eq!(loaded.config.provider.default, "anthropic");
    assert_eq!(loaded.config.providers.anthropic.model, "claude-sonnet-4-6");
    // Sources still record the global file for known leaves.
    assert!(matches!(
        loaded.sources.get("providers.anthropic.model"),
        Some(Source::Global(_)),
    ));
}

#[test]
fn malformed_global_surfaces_file_path() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = tmp.path().join("bad.toml");
    write(&bad, "not = valid = toml");

    let err = Layered::new().with_global_file(&bad).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("bad.toml") || msg.contains(bad.to_string_lossy().as_ref()),
        "error should mention the failing path; got: {msg}",
    );
}

#[test]
fn empty_files_yield_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global.toml");
    let repo = tmp.path().join(".commitcrafter.toml");
    write(&global, "");
    write(&repo, "");

    let loaded = Layered::new()
        .with_global_file(&global)
        .unwrap()
        .with_repo_file(&repo)
        .unwrap()
        .load()
        .unwrap();

    assert_eq!(loaded.config, commitcrafter::config::Config::default());
    // Every leaf still tagged as Default since nothing overrode anything.
    for (path, src) in loaded.sources.iter() {
        assert_eq!(src, &Source::Default, "leaf {path} unexpectedly {src:?}");
    }
}

#[test]
fn sources_iterate_in_alphabetical_order() {
    let loaded = Layered::new().load().unwrap();
    let paths: Vec<&str> = loaded.sources.iter().map(|(k, _)| k.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "sources should iterate in sorted order");
}
