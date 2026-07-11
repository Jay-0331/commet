//! `cc history` — inspect the accepted commit messages the tool has
//! recorded, newest first.
//!
//! Reads through the learning [`Store`] resolved from `[learning].scope`
//! (or forced to the per-repo store with `--repo`), sorts newest-first
//! by timestamp, limits to `--last N`, and renders a one-line-per-entry
//! table or `--json` for scripting.

use std::path::{Path, PathBuf};

use crate::cli::HistoryArgs;
use crate::config::Config;
use crate::config::schema::LearningScope;
use crate::error::Result;
use crate::learning::{LearningRecord, Store};

/// Run `cc history`.
pub fn run(config: &Config, args: &HistoryArgs, repo_root: Option<&Path>) -> Result<()> {
    // `--repo` forces the per-repo store regardless of the config scope.
    let scope = if args.repo {
        LearningScope::Repo
    } else {
        config.learning.scope
    };
    let store = Store::open(scope, repo_root);

    let records = select(store.read()?, args.last);

    if records.is_empty() {
        println!(
            "{}",
            empty_hint(config.learning.enabled, scope, &store.paths())
        );
        return Ok(());
    }

    let out = if args.json {
        render_json(&records)
    } else {
        render_text(&records)
    };
    print!("{out}");
    if !out.ends_with('\n') {
        println!();
    }
    Ok(())
}

/// Sort newest-first (timestamps are ISO-8601, so lexicographic order
/// is chronological) and keep at most `last` entries.
fn select(mut records: Vec<LearningRecord>, last: usize) -> Vec<LearningRecord> {
    records.sort_by(|a, b| b.ts.cmp(&a.ts));
    records.truncate(last);
    records
}

/// First line of the accepted message.
fn subject(record: &LearningRecord) -> &str {
    record.edited_text.lines().next().unwrap_or_default()
}

/// One line per record: `ts · provider/model · subject`.
fn render_text(records: &[LearningRecord]) -> String {
    records
        .iter()
        .map(|r| format!("{} · {}/{} · {}", r.ts, r.provider, r.model, subject(r)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_json(records: &[LearningRecord]) -> String {
    serde_json::to_string_pretty(records).expect("learning records serialize to JSON")
}

/// Message shown when the selected store has no entries.
fn empty_hint(enabled: bool, scope: LearningScope, paths: &[PathBuf]) -> String {
    if !enabled || scope == LearningScope::Off {
        return "No history yet — learning is disabled. \
             Enable it with `[learning].enabled = true` and a `[learning].scope` \
             of repo, global, or repo+global."
            .to_string();
    }

    let mut msg = String::from(
        "No history yet. Accepted commit messages are recorded here once you commit with `commet`.",
    );
    if !paths.is_empty() {
        msg.push_str("\nStore: ");
        let joined = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        msg.push_str(&joined);
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::{self, Store};
    use tempfile::tempdir;

    fn record(ts: &str, subject: &str) -> LearningRecord {
        LearningRecord {
            ts: ts.into(),
            repo: "commet".into(),
            branch: "main".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4-6".into(),
            format: "conventional".into(),
            candidates: vec![subject.into()],
            accepted_index: 0,
            edited_text: subject.into(),
            files: vec![],
            diff_bytes: 0,
            diff: String::new(),
        }
    }

    #[test]
    fn select_orders_newest_first_and_limits() {
        let records = (1..=5)
            .map(|i| record(&format!("2026-07-11T00:00:0{i}Z"), &format!("msg {i}")))
            .collect();
        let got = select(records, 3);

        assert_eq!(got.len(), 3);
        assert_eq!(got[0].ts, "2026-07-11T00:00:05Z");
        assert_eq!(got[1].ts, "2026-07-11T00:00:04Z");
        assert_eq!(got[2].ts, "2026-07-11T00:00:03Z");
    }

    #[test]
    fn write_five_then_history_last_three() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".commet").join("history.jsonl");
        for i in 1..=5 {
            learning::append(
                &path,
                &record(&format!("2026-07-11T00:00:0{i}Z"), &format!("m{i}")),
            )
            .unwrap();
        }
        let store = Store::with_paths(LearningScope::Repo, Some(path), None);

        let got = select(store.read().unwrap(), 3);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].edited_text, "m5"); // newest first
        assert_eq!(got[2].edited_text, "m3");
    }

    #[test]
    fn render_text_has_expected_shape() {
        let line = render_text(&[record("2026-07-11T00:00:00Z", "feat: thing\nbody")]);
        assert_eq!(
            line,
            "2026-07-11T00:00:00Z · anthropic/claude-sonnet-4-6 · feat: thing"
        );
    }

    #[test]
    fn render_json_is_parseable() {
        let json = render_json(&[record("2026-07-11T00:00:00Z", "feat: x")]);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["edited_text"], "feat: x");
        assert_eq!(parsed[0]["provider"], "anthropic");
    }

    #[test]
    fn empty_hint_mentions_enabling_when_off() {
        let hint = empty_hint(false, LearningScope::Off, &[]);
        assert!(hint.contains("[learning].enabled"));
        assert!(hint.contains("disabled"));
    }

    #[test]
    fn empty_hint_shows_store_path_when_enabled() {
        let hint = empty_hint(
            true,
            LearningScope::Repo,
            &[PathBuf::from("/work/.commet/history.jsonl")],
        );
        assert!(hint.contains("/work/.commet/history.jsonl"));
        assert!(hint.contains("commet"));
    }
}
