//! `Provider` trait, request/error types, and the builtin registry.
//!
//! Every LLM backend (Anthropic, OpenAI, OpenRouter, Ollama) implements
//! [`Provider`] so the rest of the app — CLI flags, TUI, learning loop —
//! stays provider-agnostic and only ever talks to `Box<dyn Provider>`.

use std::collections::HashMap;
use std::time::Duration;

use thiserror::Error;

mod base;
pub use base::HttpClient;

mod openai_compat;
pub use openai_compat::{ChatMessage, ChatRequest, complete};

mod openai;
pub use openai::OpenAiProvider;

/// Input to a single generation call. `n > 1` asks the adapter for that
/// many independent candidate messages from the same diff.
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub n: u8,
}

/// Failure modes shared by every adapter, independent of transport.
///
/// Variants are unit-like (aside from the two that carry data control
/// flow needs) because message text comes from the HTTP layer, not
/// from this enum — [`RateLimited`](Self::RateLimited) carries
/// `retry_after` for backoff and [`BadResponse`](Self::BadResponse)
/// carries a snippet for diagnostics; the rest just need to be
/// matched on.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("missing API key")]
    MissingKey,

    #[error("unauthorized (check your API key)")]
    Unauthorized,

    #[error("rate limited (retry_after: {retry_after:?})")]
    RateLimited { retry_after: Option<Duration> },

    #[error("request timed out")]
    Timeout,

    #[error("network error")]
    Network,

    #[error("unexpected response: {snippet}")]
    BadResponse { snippet: String },
}

/// A single LLM backend. Implementations are stateless — every call
/// takes the full request it needs, so a `Box<dyn Provider>` can be
/// shared across threads and multi-candidate calls freely.
pub trait Provider: Send + Sync {
    /// Registry key / config section name, e.g. `"anthropic"`.
    fn name(&self) -> &'static str;

    /// Environment variable holding the API key, or `None` for
    /// providers that don't need one (e.g. local Ollama).
    fn key_env_var(&self) -> Option<&'static str>;

    /// Generate `req.n` candidate commit messages.
    fn generate(&self, req: &GenerateRequest) -> Result<Vec<String>, ProviderError>;
}

pub struct AnthropicProvider;

impl Provider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn key_env_var(&self) -> Option<&'static str> {
        Some("ANTHROPIC_API_KEY")
    }

    fn generate(&self, _req: &GenerateRequest) -> Result<Vec<String>, ProviderError> {
        unimplemented!("Anthropic adapter lands in a follow-up issue")
    }
}

pub struct OpenRouterProvider;

impl Provider for OpenRouterProvider {
    fn name(&self) -> &'static str {
        "openrouter"
    }

    fn key_env_var(&self) -> Option<&'static str> {
        Some("OPENROUTER_API_KEY")
    }

    fn generate(&self, _req: &GenerateRequest) -> Result<Vec<String>, ProviderError> {
        unimplemented!("OpenRouter adapter lands in a follow-up issue")
    }
}

pub struct OllamaProvider;

impl Provider for OllamaProvider {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn key_env_var(&self) -> Option<&'static str> {
        None
    }

    fn generate(&self, _req: &GenerateRequest) -> Result<Vec<String>, ProviderError> {
        unimplemented!("Ollama adapter lands in a follow-up issue")
    }
}

/// The four builtin providers, keyed by [`Provider::name`].
pub fn registry() -> HashMap<&'static str, Box<dyn Provider>> {
    let providers: Vec<Box<dyn Provider>> = vec![
        Box::new(AnthropicProvider),
        Box::new(OpenAiProvider::new()),
        Box::new(OpenRouterProvider),
        Box::new(OllamaProvider),
    ];
    providers.into_iter().map(|p| (p.name(), p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_all_four_builtin_providers() {
        let reg = registry();
        for key in ["anthropic", "openai", "openrouter", "ollama"] {
            assert!(reg.contains_key(key), "registry missing {key}");
        }
        assert_eq!(reg.len(), 4);
    }

    #[test]
    fn provider_name_matches_its_registry_key() {
        let reg = registry();
        for (key, provider) in &reg {
            assert_eq!(*key, provider.name());
        }
    }

    #[test]
    fn only_ollama_has_no_key_env_var() {
        let reg = registry();
        for (key, provider) in &reg {
            let has_key = provider.key_env_var().is_some();
            assert_eq!(
                has_key,
                *key != "ollama",
                "unexpected key_env_var for {key}"
            );
        }
    }
}
