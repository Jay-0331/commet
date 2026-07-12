//! `cc forget` — wipe learning history at a chosen granularity.
//!
//! `--all` clears both stores, `--repo` clears only the per-repo store
//! (both are destructive and prompt for a typed `yes` unless `-y`), and
//! `--last` drops just the most recent record from the configured
//! scope. Exits 1 (`UserAbort`) when the user declines or nothing
//! matched.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::cli::ForgetArgs;
use crate::config::Config;
use crate::config::schema::LearningScope;
use crate::error::{Error, Result};
use crate::learning::{self, Store};

/// Which files a forget targets and how.
enum Plan {
    /// Truncate every file in `paths` (destructive → needs confirm).
    Clear {
        paths: Vec<PathBuf>,
        destructive: bool,
    },
    /// Drop the newest record across `paths`.
    DropLast { paths: Vec<PathBuf> },
}

/// Run `cc forget`, prompting for confirmation on the terminal.
pub fn run(config: &Config, args: &ForgetArgs, cwd: &Path) -> Result<()> {
    execute(plan(config, args, cwd), args.yes, prompt_confirm)
}

/// Resolve which store files this invocation touches.
fn plan(config: &Config, args: &ForgetArgs, cwd: &Path) -> Plan {
    let repo_root = crate::git::repo_root(cwd).ok();
    let paths = |scope| Store::open(scope, repo_root.as_deref()).paths();

    if args.all {
        Plan::Clear {
            paths: paths(LearningScope::RepoGlobal),
            destructive: true,
        }
    } else if args.repo {
        Plan::Clear {
            paths: paths(LearningScope::Repo),
            destructive: true,
        }
    } else {
        // `--last` (clap's ArgGroup guarantees exactly one flag).
        Plan::DropLast {
            paths: paths(config.learning.scope),
        }
    }
}

/// Carry out a [`Plan`], gating destructive clears behind `confirm`
/// (skipped when `yes`). Errors with `UserAbort` (exit 1) on a declined
/// prompt or when nothing matched.
fn execute(plan: Plan, yes: bool, mut confirm: impl FnMut() -> bool) -> Result<()> {
    match plan {
        Plan::Clear { paths, destructive } => {
            if destructive && !yes && !confirm() {
                return Err(Error::UserAbort);
            }
            let mut cleared = 0usize;
            for path in &paths {
                if learning::clear(path)? {
                    cleared += 1;
                }
            }
            if cleared == 0 {
                eprintln!("nothing to forget");
                return Err(Error::UserAbort);
            }
            println!("Forgot learning history ({cleared} file(s) cleared).");
            Ok(())
        }
        Plan::DropLast { paths } => {
            if learning::drop_last(&paths)? {
                println!("Forgot the most recent entry.");
                Ok(())
            } else {
                eprintln!("nothing to forget");
                Err(Error::UserAbort)
            }
        }
    }
}

/// Prompt on stderr and read a typed `yes` from stdin.
fn prompt_confirm() -> bool {
    eprint!("This permanently deletes learning history. Type 'yes' to confirm: ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin().read_line(&mut line).is_ok() && line.trim() == "yes"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::{self, LearningRecord};
    use tempfile::tempdir;

    fn record(ts: &str, text: &str) -> LearningRecord {
        LearningRecord {
            ts: ts.into(),
            repo: "r".into(),
            branch: "main".into(),
            provider: "anthropic".into(),
            model: "m".into(),
            format: "conventional".into(),
            candidates: vec![text.into()],
            accepted_index: 0,
            edited_text: text.into(),
            files: vec![],
            diff_bytes: 0,
            diff: String::new(),
        }
    }

    fn seeded() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        for i in 1..=3 {
            learning::append(
                &path,
                &record(&format!("2026-07-12T00:00:0{i}Z"), &format!("m{i}")),
            )
            .unwrap();
        }
        (dir, path)
    }

    #[test]
    fn clear_all_with_yes_empties_the_file() {
        let (_d, path) = seeded();
        execute(
            Plan::Clear {
                paths: vec![path.clone()],
                destructive: true,
            },
            true,
            || panic!("confirm must not be called with yes"),
        )
        .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn drop_last_removes_only_the_last() {
        let (_d, path) = seeded();
        execute(
            Plan::DropLast {
                paths: vec![path.clone()],
            },
            false,
            || true,
        )
        .unwrap();
        let left = learning::load(&path).unwrap();
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|r| r.edited_text != "m3"));
    }

    #[test]
    fn declined_confirmation_aborts_without_clearing() {
        let (_d, path) = seeded();
        let err = execute(
            Plan::Clear {
                paths: vec![path.clone()],
                destructive: true,
            },
            false,
            || false, // user typed something other than yes
        )
        .unwrap_err();
        assert!(matches!(err, Error::UserAbort));
        assert!(path.exists(), "declining must leave the store intact");
    }

    #[test]
    fn nothing_to_forget_exits_with_abort() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("history.jsonl");
        let err = execute(
            Plan::Clear {
                paths: vec![missing],
                destructive: true,
            },
            true,
            || true,
        )
        .unwrap_err();
        assert!(matches!(err, Error::UserAbort));

        let dir2 = tempdir().unwrap();
        let err2 = execute(
            Plan::DropLast {
                paths: vec![dir2.path().join("h.jsonl")],
            },
            true,
            || true,
        )
        .unwrap_err();
        assert!(matches!(err2, Error::UserAbort));
    }
}
