//! Where each configuration leaf came from.
//!
//! Every key in a loaded [`crate::config::Config`] is annotated with
//! a [`Source`] so `cc config show` can answer "why is `style.format`
//! set to `gitmoji`?" without the user grepping their dotfiles.
//!
//! Sources are stored in a [`Sources`] map keyed by dotted TOML path
//! (`style.format`, `providers.openai.model`, etc.). The map uses
//! [`BTreeMap`] so iteration order is alphabetical and stable across
//! runs — handy for snapshot tests and for human-readable output.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a single configuration leaf came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Baked-in default from [`crate::config::Config::default`].
    Default,
    /// Loaded from the global config file at this path.
    Global(PathBuf),
    /// Loaded from the per-repo config file at this path.
    Repo(PathBuf),
    /// Set by a CLI flag (e.g. `--provider`, `--model`, `--type`,
    /// `--no-color`).
    Flag,
    /// Set by a `--set <key.path>=<value>` override.
    Set,
}

impl Source {
    /// Short human-readable label used in `cc config show`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Global(_) => "global",
            Self::Repo(_) => "repo",
            Self::Flag => "flag",
            Self::Set => "--set",
        }
    }

    /// Path to the file that contributed this leaf, if any.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Global(p) | Self::Repo(p) => Some(p.as_path()),
            Self::Default | Self::Flag | Self::Set => None,
        }
    }
}

/// Map from dotted TOML path to its winning [`Source`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources {
    map: BTreeMap<String, Source>,
}

impl Sources {
    /// Empty map — defaults haven't been recorded yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set or overwrite the source for a specific path.
    pub fn set(&mut self, path: impl Into<String>, source: Source) {
        self.map.insert(path.into(), source);
    }

    /// Look up the source for a specific path. Returns `None` for any
    /// path the loader never wrote (typically because the path doesn't
    /// exist in the schema).
    pub fn get(&self, path: &str) -> Option<&Source> {
        self.map.get(path)
    }

    /// Iterate every (path, source) pair in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Source)> {
        self.map.iter()
    }

    /// Total number of recorded leaves. Used by tests.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `true` when nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_documented_strings() {
        assert_eq!(Source::Default.label(), "default");
        assert_eq!(Source::Global(PathBuf::from("/a")).label(), "global");
        assert_eq!(Source::Repo(PathBuf::from("/b")).label(), "repo");
        assert_eq!(Source::Flag.label(), "flag");
        assert_eq!(Source::Set.label(), "--set");
    }

    #[test]
    fn path_returns_some_only_for_file_sources() {
        assert!(Source::Default.path().is_none());
        assert!(Source::Flag.path().is_none());
        assert!(Source::Set.path().is_none());
        assert_eq!(
            Source::Global(PathBuf::from("/g.toml")).path(),
            Some(Path::new("/g.toml")),
        );
        assert_eq!(
            Source::Repo(PathBuf::from("/r.toml")).path(),
            Some(Path::new("/r.toml")),
        );
    }

    #[test]
    fn sources_set_overwrites_previous() {
        let mut s = Sources::new();
        s.set("style.format", Source::Default);
        s.set("style.format", Source::Set);
        assert_eq!(s.get("style.format"), Some(&Source::Set));
    }

    #[test]
    fn sources_iterates_alphabetically() {
        let mut s = Sources::new();
        s.set("zeta", Source::Default);
        s.set("alpha", Source::Default);
        s.set("middle", Source::Default);
        let keys: Vec<&str> = s.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["alpha", "middle", "zeta"]);
    }
}
