//! Spawn the user's preferred text editor on a file.
//!
//! Resolves the editor name in the standard order:
//!
//! 1. `$VISUAL` if set and non-empty
//! 2. `$EDITOR` if set and non-empty
//! 3. `vi` as a last-resort fallback
//!
//! Then runs `<editor> <path>` and waits for it to exit. Any non-zero
//! exit status surfaces as [`Error::Config`] so the caller can route
//! it to the standard exit-code mapping in `main`.
//!
//! Tests use [`resolve_from`] to drive the resolution rules without
//! touching process-wide environment variables; the public
//! [`resolve`] wraps it with the real `std::env`.

use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

/// Look up the editor to run, consulting `$VISUAL` then `$EDITOR`,
/// falling back to `vi`.
pub fn resolve() -> String {
    resolve_from(|k| std::env::var(k).ok())
}

/// Pure variant of [`resolve`] used by tests; takes an env lookup so
/// callers can stub `VISUAL` / `EDITOR` without touching the process.
pub fn resolve_from<F: Fn(&str) -> Option<String>>(lookup: F) -> String {
    if let Some(v) = lookup("VISUAL")
        && !v.is_empty()
    {
        return v;
    }
    if let Some(v) = lookup("EDITOR")
        && !v.is_empty()
    {
        return v;
    }
    "vi".to_string()
}

/// Spawn `<editor> <path>` and wait for it to exit. Non-zero exit
/// status produces an [`Error::Config`].
pub fn spawn(path: &Path) -> Result<()> {
    let editor = resolve();
    let status = Command::new(&editor).arg(path).status().map_err(|e| {
        Error::Config(format!(
            "failed to spawn editor `{editor}` on {}: {e}",
            path.display(),
        ))
    })?;
    if !status.success() {
        return Err(Error::Config(format!(
            "editor `{editor}` exited with non-zero status: {status}",
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(map: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k: &str| {
            map.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn visual_wins_over_editor() {
        let env = lookup(&[("VISUAL", "code -w"), ("EDITOR", "nano")]);
        assert_eq!(resolve_from(env), "code -w");
    }

    #[test]
    fn editor_wins_when_visual_unset() {
        let env = lookup(&[("EDITOR", "nano")]);
        assert_eq!(resolve_from(env), "nano");
    }

    #[test]
    fn empty_visual_falls_through_to_editor() {
        let env = lookup(&[("VISUAL", ""), ("EDITOR", "nano")]);
        assert_eq!(resolve_from(env), "nano");
    }

    #[test]
    fn empty_editor_falls_through_to_vi() {
        let env = lookup(&[("VISUAL", ""), ("EDITOR", "")]);
        assert_eq!(resolve_from(env), "vi");
    }

    #[test]
    fn both_unset_falls_through_to_vi() {
        let env = lookup(&[]);
        assert_eq!(resolve_from(env), "vi");
    }
}
