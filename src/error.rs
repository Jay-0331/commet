//! Crate-wide error type and process exit-code mapping.
//!
//! Every fallible operation in `commitcrafter` returns
//! [`Result<T>`](Result), which uses the [`Error`] enum below. `main` walks
//! that error and converts it to a process [`ExitCode`] via
//! [`Error::exit_code`] so the mapping lives in one place.

use std::process::ExitCode;

use thiserror::Error;

/// Crate-wide error type.
///
/// Variants are deliberately coarse; per-domain errors stringify into the
/// corresponding variant rather than nesting their own enums. The
/// `From<std::io::Error>` impl is the only automatic conversion — every
/// other call-site converts explicitly so the variant choice (and therefore
/// the resulting exit code) is intentional.
#[derive(Debug, Error)]
pub enum Error {
    /// Configuration is missing, malformed, or contains an invalid value.
    #[error("config error: {0}")]
    Config(String),

    /// A `git` subprocess failed or produced output we could not parse.
    #[error("git error: {0}")]
    Git(String),

    /// A provider HTTP call failed, returned a non-success status, or
    /// returned a body we could not parse.
    #[error("provider error: {0}")]
    Provider(String),

    /// Raw I/O error from the standard library; usually wrapped into a more
    /// specific variant at the call-site but available for cases where
    /// none applies.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// `cc doctor` / `cc setup` reported at least one failing check.
    #[error("health check failed")]
    Doctor,

    /// The user explicitly aborted (quit the TUI, declined a prompt,
    /// pressed Ctrl-C in a confirmation, etc.).
    #[error("user aborted")]
    UserAbort,
}

impl Error {
    /// Process exit code per the plan's mapping table:
    ///
    /// | code | meaning                |
    /// |------|------------------------|
    /// | `0`  | success (not produced here) |
    /// | `1`  | user abort             |
    /// | `2`  | git error              |
    /// | `3`  | provider error         |
    /// | `4`  | config error           |
    /// | `5`  | doctor / setup failed  |
    ///
    /// I/O errors fall through to `2` (treated as git-adjacent until a
    /// downstream call-site reclassifies them with a more specific
    /// variant).
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::UserAbort => 1,
            Self::Git(_) | Self::Io(_) => 2,
            Self::Provider(_) => 3,
            Self::Config(_) => 4,
            Self::Doctor => 5,
        }
    }
}

impl From<&Error> for ExitCode {
    fn from(err: &Error) -> Self {
        ExitCode::from(err.exit_code())
    }
}

/// Shorthand for the crate's `Result` alias.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_matches_plan() {
        assert_eq!(Error::UserAbort.exit_code(), 1);
        assert_eq!(Error::Git("boom".into()).exit_code(), 2);
        assert_eq!(Error::Provider("boom".into()).exit_code(), 3);
        assert_eq!(Error::Config("boom".into()).exit_code(), 4);
        assert_eq!(Error::Doctor.exit_code(), 5);
    }

    #[test]
    fn io_error_maps_to_git_adjacent_code() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err: Error = io.into();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn display_includes_inner_message() {
        let err = Error::Config("missing [provider] section".into());
        assert_eq!(err.to_string(), "config error: missing [provider] section");

        let err = Error::Provider("401 unauthorized".into());
        assert_eq!(err.to_string(), "provider error: 401 unauthorized");
    }

    #[test]
    fn user_abort_has_static_message() {
        assert_eq!(Error::UserAbort.to_string(), "user aborted");
    }

    #[test]
    fn doctor_has_static_message() {
        assert_eq!(Error::Doctor.to_string(), "health check failed");
    }
}
