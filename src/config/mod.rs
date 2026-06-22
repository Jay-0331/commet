//! Configuration loading and merging.
//!
//! The crate's configuration is layered:
//!
//! 1. **Defaults** — values baked into [`schema::Config::default`].
//! 2. **Global** — `$XDG_CONFIG_HOME/commitcrafter/config.toml` (or
//!    `~/.config/commitcrafter/config.toml`).
//! 3. **Repo** — `<repo-root>/.commitcrafter.toml`, discovered via
//!    `git rev-parse --show-toplevel`.
//! 4. **CLI flags** — values derived from `--provider`, `--model`,
//!    `--no-color`, `--type`. Constructed by the caller, fed in as a
//!    raw [`toml::Value`].
//! 5. **`--set <key.path>=<value>`** overrides — repeatable, never
//!    persisted, highest precedence.
//!
//! Each leaf in the final [`Config`] also records its [`Source`] so
//! `cc config show` can annotate every key with where it came from.
//!
//! The public entry point for application code is [`Layered`].
//! Tests typically construct one manually:
//!
//! ```no_run
//! use commitcrafter::config::{Layered, Source};
//!
//! let loaded = Layered::new()
//!     .with_global_file("/tmp/global.toml")
//!     .expect("global parsed")
//!     .with_repo_file("/tmp/repo.toml")
//!     .expect("repo parsed")
//!     .load()
//!     .expect("merge succeeded");
//!
//! // Every config key is now reachable on `loaded.config`, and every
//! // leaf path is annotated in `loaded.sources`.
//! assert_eq!(loaded.config.provider.default, "anthropic");
//! ```

pub mod discover;
pub mod merge;
pub mod schema;
pub mod source;

pub use merge::{Layered, Loaded};
pub use schema::{
    AnthropicConfig, ColorMode, Config, CustomColors, CustomStyle, Git, Learning, LearningScope,
    MessageFormat, OllamaConfig, OpenAiConfig, OpenRouterConfig, ProviderSelection, Providers,
    Style, ThemeName, Ui, find_unknown_keys,
};
pub use source::{Source, Sources};
