//! Deterministic, offline provider for integration tests.
//!
//! Compiled only under `cfg(test)` or the `mock` feature. When
//! `$COMMET_MOCK_RESPONSE` is set, [`super::registry`] (with the
//! `mock` feature) returns this provider under every provider name, so
//! the default generate flow runs end to end without touching the
//! network. Controlled entirely by environment variables:
//!
//! - `COMMET_MOCK_RESPONSE` — newline-separated candidates
//!   (one line per candidate, so `-g 3` works).
//! - `COMMET_MOCK_LOG` — optional path; the last
//!   [`GenerateRequest`] is written there as JSON for prompt assertions.
//! - `COMMET_MOCK_DELAY_MS` — optional per-call sleep so spinner
//!   tests can observe multiple frames.

use std::collections::HashMap;
use std::time::Duration;

use super::{GenerateRequest, Provider, ProviderError};

/// Env var holding the newline-separated candidate list.
pub const RESPONSE_ENV: &str = "COMMET_MOCK_RESPONSE";
/// Env var naming the file to record the last request to.
pub const LOG_ENV: &str = "COMMET_MOCK_LOG";
/// Env var setting a per-call delay in milliseconds.
pub const DELAY_ENV: &str = "COMMET_MOCK_DELAY_MS";

/// Offline provider driven by environment variables.
pub struct MockProvider;

impl Provider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn key_env_var(&self) -> Option<&'static str> {
        None
    }

    fn generate(&self, req: &GenerateRequest) -> Result<Vec<String>, ProviderError> {
        if let Ok(ms) = std::env::var(DELAY_ENV)
            && let Ok(ms) = ms.parse::<u64>()
        {
            std::thread::sleep(Duration::from_millis(ms));
        }

        if let Ok(path) = std::env::var(LOG_ENV) {
            let json = serde_json::to_string(req).map_err(|e| ProviderError::BadResponse {
                snippet: e.to_string(),
            })?;
            std::fs::write(path, json).map_err(|_| ProviderError::Network)?;
        }

        let response = std::env::var(RESPONSE_ENV).map_err(|_| ProviderError::BadResponse {
            snippet: format!("${RESPONSE_ENV} is not set"),
        })?;

        let mut candidates: Vec<String> = response
            .split('\n')
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();

        let requested = usize::from(req.n.max(1));
        if candidates.len() < requested {
            return Err(ProviderError::BadResponse {
                snippet: format!(
                    "${RESPONSE_ENV} provided {} candidate(s), but {requested} requested",
                    candidates.len()
                ),
            });
        }
        candidates.truncate(requested);
        Ok(candidates)
    }
}

/// A registry that resolves every builtin provider name — plus `mock`
/// itself — to a [`MockProvider`], so config-selected provider names
/// still find the mock.
pub fn registry() -> HashMap<&'static str, Box<dyn Provider>> {
    ["anthropic", "openai", "openrouter", "ollama", "mock"]
        .into_iter()
        .map(|name| (name, Box::new(MockProvider) as Box<dyn Provider>))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "sys".into(),
            user_prompt: "usr".into(),
            model: "mock-model".into(),
            max_tokens: 128,
            temperature: 0.1,
            n: 3,
        }
    }

    #[test]
    fn name_and_no_key() {
        let m = MockProvider;
        assert_eq!(m.name(), "mock");
        assert_eq!(m.key_env_var(), None);
    }

    #[test]
    fn registry_maps_every_provider_name_to_mock() {
        let reg = registry();
        for name in ["anthropic", "openai", "openrouter", "ollama", "mock"] {
            assert_eq!(reg.get(name).unwrap().name(), "mock");
        }
    }

    /// One test owns all the env-var mutation so mock runs don't race
    /// each other on these process-global variables.
    #[test]
    fn generate_splits_candidates_logs_request_and_delays() {
        // Serialize against the builtin-registry tests: under the `mock`
        // feature they read these same env vars through `registry()`.
        let _g = crate::provider::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("req.json");

        // SAFETY: this is the only test that touches these vars, and it
        // sets, uses, and clears them within one thread.
        unsafe {
            std::env::set_var(
                RESPONSE_ENV,
                "feat: a\nfix: b\nchore: c\ndocs: ignored extra",
            );
            std::env::set_var(LOG_ENV, &log);
            std::env::set_var(DELAY_ENV, "20");
        }

        let start = std::time::Instant::now();
        let candidates = MockProvider.generate(&request()).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(candidates, vec!["feat: a", "fix: b", "chore: c"]);
        assert!(elapsed >= Duration::from_millis(20), "delay not honored");

        // The last request was recorded as JSON for prompt assertions.
        let logged = std::fs::read_to_string(&log).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&logged).unwrap();
        assert_eq!(parsed["system_prompt"], "sys");
        assert_eq!(parsed["n"], 3);

        // The fixture must supply at least the requested count; this keeps
        // end-to-end tests honest about whether `-g N` reached the provider.
        unsafe {
            std::env::set_var(RESPONSE_ENV, "feat: only one");
        }
        let err = MockProvider.generate(&request()).unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse { .. }));

        unsafe {
            std::env::remove_var(RESPONSE_ENV);
            std::env::remove_var(LOG_ENV);
            std::env::remove_var(DELAY_ENV);
        }

        // With the response var cleared, generate reports a BadResponse
        // rather than hanging or panicking. (Kept in this one test so
        // all mutation of the process-global env stays single-threaded.)
        let err = MockProvider.generate(&request()).unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse { .. }));
    }
}
