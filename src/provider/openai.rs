//! OpenAI adapter — thin glue over [`super::openai_compat`].
//!
//! OpenAI speaks the same chat-completions dialect as OpenRouter, so
//! all the wire-format work lives in `openai_compat`; this module
//! only owns the base URL, auth header, and key lookup.

use std::time::Duration;

use super::{ChatMessage, ChatRequest, GenerateRequest, HttpClient, Provider, ProviderError};

const BASE_URL: &str = "https://api.openai.com/v1";

pub struct OpenAiProvider {
    client: HttpClient,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new() -> Self {
        Self {
            client: HttpClient::new(Duration::from_secs(30), 2),
            base_url: BASE_URL.into(),
        }
    }

    #[cfg(test)]
    fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: HttpClient::new(Duration::from_secs(5), 0),
            base_url: base_url.into(),
        }
    }

    /// Does the actual request/response work against `api_key`.
    /// Split out from [`Provider::generate`] so tests can supply a
    /// key directly instead of racing on process-global env vars.
    fn generate_with_key(
        &self,
        req: &GenerateRequest,
        api_key: &str,
    ) -> Result<Vec<String>, ProviderError> {
        let auth = format!("Bearer {api_key}");
        let headers = [("Authorization", auth.as_str())];
        let chat_req = ChatRequest {
            model: req.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: req.system_prompt.clone(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: req.user_prompt.clone(),
                },
            ],
            n: req.n,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
        };
        super::complete(&self.client, &self.base_url, &headers, &chat_req, req.n)
    }
}

impl Default for OpenAiProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn key_env_var(&self) -> Option<&'static str> {
        Some("OPENAI_API_KEY")
    }

    fn generate(&self, req: &GenerateRequest) -> Result<Vec<String>, ProviderError> {
        let key = std::env::var("OPENAI_API_KEY").map_err(|_| ProviderError::MissingKey)?;
        self.generate_with_key(req, &key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn req() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "system instructions".into(),
            user_prompt: "diff text".into(),
            model: "gpt-4o-mini".into(),
            max_tokens: 1024,
            temperature: 0.2,
            n: 1,
        }
    }

    #[tokio::test]
    async fn generate_posts_expected_url_auth_header_and_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "feat: add x"}}]
            })))
            .mount(&server)
            .await;

        let base_url = server.uri();
        let request = req();

        let candidates = tokio::task::spawn_blocking(move || {
            let provider = OpenAiProvider::with_base_url(base_url);
            provider.generate_with_key(&request, "test-key").unwrap()
        })
        .await
        .unwrap();

        assert_eq!(candidates, vec!["feat: add x".to_string()]);

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let request = &received[0];
        assert_eq!(request.url.path(), "/chat/completions");
        assert_eq!(
            request.headers.get("authorization").unwrap(),
            "Bearer test-key"
        );

        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["n"], 1);
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "system instructions");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "diff text");
    }

    #[tokio::test]
    async fn generate_propagates_provider_error_from_transport() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let base_url = server.uri();
        let request = req();

        let err = tokio::task::spawn_blocking(move || {
            let provider = OpenAiProvider::with_base_url(base_url);
            provider.generate_with_key(&request, "bad-key").unwrap_err()
        })
        .await
        .unwrap();

        assert!(matches!(err, ProviderError::Unauthorized));
    }

    #[test]
    fn name_and_key_env_var_match_openai() {
        let provider = OpenAiProvider::new();
        assert_eq!(provider.name(), "openai");
        assert_eq!(provider.key_env_var(), Some("OPENAI_API_KEY"));
    }
}
