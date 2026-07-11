//! `tracing` initialization.
//!
//! Reads the log filter from `COMMET_LOG` (default `warn`) and
//! installs a single global subscriber that writes to stderr.
//!
//! When the ratatui TUI lands (E5) this module will gain a second entry
//! point that routes to a log file under `$XDG_STATE_HOME/commet/`
//! so the screen stays clean. For now the binary never enters the TUI, so
//! stderr is the only sink we need.

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;

/// Environment variable consulted for the log filter (e.g.
/// `COMMET_LOG=info,reqwest=warn`).
pub const ENV_VAR: &str = "COMMET_LOG";

/// Default filter when [`ENV_VAR`] is unset or empty.
pub const DEFAULT_FILTER: &str = "warn";

/// Install the stderr `tracing` subscriber.
///
/// Call once from `main`. Subsequent calls are a no-op (the global
/// subscriber can only be set once); we swallow the resulting error so
/// tests that init their own subscriber don't crash on re-entry.
pub fn init_stderr() {
    let filter =
        EnvFilter::try_from_env(ENV_VAR).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_and_default_match_documentation() {
        // Locks the contract advertised in `cc --help` and the README.
        assert_eq!(ENV_VAR, "COMMET_LOG");
        assert_eq!(DEFAULT_FILTER, "warn");
    }

    #[test]
    fn init_is_idempotent() {
        // Calling twice must not panic; the global subscriber can only be
        // set once but `try_init` returns an Err we deliberately ignore.
        init_stderr();
        init_stderr();
    }

    #[test]
    fn default_filter_parses_as_envfilter() {
        // Catches the case where someone changes `DEFAULT_FILTER` to
        // something `EnvFilter::new` would reject at runtime.
        let _ = EnvFilter::new(DEFAULT_FILTER);
    }
}
