//! Anthropic adapter — the Messages API has its own wire shape and no
//! `n` parameter, so unlike OpenAI/OpenRouter it can't reuse
//! [`super::openai_compat`].
//!
//! Multi-candidate generation fans `req.n` identical requests out over
//! `std::thread::scope` — the shared [`HttpClient`] is blocking
//! (`reqwest::blocking`), so parallelism is threads, not `tokio`.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{GenerateRequest, HttpClient, Provider, ProviderError};

const BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// A single message in the Messages API `messages` array.
#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

/// Request body for `POST {base_url}/messages`. `system` is a
/// top-level field, distinct from the `messages` array.
#[derive(Debug, Serialize)]
struct MessagesRequest {
    model: String,
    system: String,
    messages: Vec<Message>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
}

/// One block of the response `content` array. Only `text` blocks carry
/// a commit message; `type` lets us skip any other block kind (e.g. a
/// `thinking` block) that might precede it.
#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

pub struct AnthropicProvider {
    client: HttpClient,
    base_url: String,
}

impl AnthropicProvider {
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
        let n = req.n.max(1);
        let headers = [
            ("x-api-key", api_key),
            ("anthropic-version", ANTHROPIC_VERSION),
        ];

        // The Messages API has no `n`; fan out one request per candidate
        // and collect them. Scoped threads let each borrow `self` and
        // `headers` without cloning.
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..n)
                .map(|_| scope.spawn(|| self.one_candidate(&headers, req)))
                .collect();

            handles
                .into_iter()
                .map(|h| h.join().expect("candidate request thread panicked"))
                .collect()
        })
    }

    /// Issue one Messages API call and pull the first `text` block.
    fn one_candidate(
        &self,
        headers: &[(&str, &str)],
        req: &GenerateRequest,
    ) -> Result<String, ProviderError> {
        let url = format!("{}/messages", self.base_url);
        let body = MessagesRequest {
            model: req.model.clone(),
            system: req.system_prompt.clone(),
            messages: vec![Message {
                role: "user".into(),
                content: req.user_prompt.clone(),
            }],
            max_tokens: req.max_tokens,
            temperature: req.temperature,
        };

        let resp: MessagesResponse = self.client.post_json(&url, headers, &body)?;

        resp.content
            .into_iter()
            .find(|block| block.block_type == "text")
            .map(|block| block.text)
            .ok_or_else(|| ProviderError::BadResponse {
                snippet: "response contained no text block".into(),
            })
    }
}

impl Default for AnthropicProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn key_env_var(&self) -> Option<&'static str> {
        Some("ANTHROPIC_API_KEY")
    }

    fn generate(&self, req: &GenerateRequest) -> Result<Vec<String>, ProviderError> {
        let key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| ProviderError::MissingKey)?;
        self.generate_with_key(req, &key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn req(n: u8) -> GenerateRequest {
        GenerateRequest {
            system_prompt: "system instructions".into(),
            user_prompt: "diff text".into(),
            model: "claude-sonnet-4-6".into(),
            max_tokens: 1024,
            temperature: 0.2,
            n,
        }
    }

    async fn mount_text(server: &MockServer, text: &str) {
        Mock::given(method("POST"))
            .and(path("/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": text}]
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn single_call_posts_expected_url_headers_and_body() {
        let server = MockServer::start().await;
        mount_text(&server, "feat: add x").await;

        let base_url = server.uri();
        let candidates = tokio::task::spawn_blocking(move || {
            let provider = AnthropicProvider::with_base_url(base_url);
            provider.generate_with_key(&req(1), "test-key").unwrap()
        })
        .await
        .unwrap();

        assert_eq!(candidates, vec!["feat: add x".to_string()]);

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let request = &received[0];
        assert_eq!(request.url.path(), "/messages");
        assert_eq!(request.headers.get("x-api-key").unwrap(), "test-key");
        assert_eq!(
            request.headers.get("anthropic-version").unwrap(),
            "2023-06-01"
        );

        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["system"], "system instructions");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "diff text");
    }

    #[tokio::test]
    async fn n_candidates_fan_out_to_n_parallel_calls() {
        let server = MockServer::start().await;
        mount_text(&server, "feat: parallel").await;

        let base_url = server.uri();
        let candidates = tokio::task::spawn_blocking(move || {
            let provider = AnthropicProvider::with_base_url(base_url);
            provider.generate_with_key(&req(3), "test-key").unwrap()
        })
        .await
        .unwrap();

        assert_eq!(candidates.len(), 3);
        assert!(candidates.iter().all(|c| c == "feat: parallel"));

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 3);
    }

    #[tokio::test]
    async fn skips_leading_non_text_block() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [
                    {"type": "thinking", "thinking": "…"},
                    {"type": "text", "text": "fix: the bug"}
                ]
            })))
            .mount(&server)
            .await;

        let base_url = server.uri();
        let candidates = tokio::task::spawn_blocking(move || {
            let provider = AnthropicProvider::with_base_url(base_url);
            provider.generate_with_key(&req(1), "test-key").unwrap()
        })
        .await
        .unwrap();

        assert_eq!(candidates, vec!["fix: the bug".to_string()]);
    }

    #[tokio::test]
    async fn no_text_block_surfaces_as_bad_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"content": []})))
            .mount(&server)
            .await;

        let base_url = server.uri();
        let err = tokio::task::spawn_blocking(move || {
            let provider = AnthropicProvider::with_base_url(base_url);
            provider.generate_with_key(&req(1), "test-key").unwrap_err()
        })
        .await
        .unwrap();

        assert!(matches!(err, ProviderError::BadResponse { .. }));
    }

    #[tokio::test]
    async fn unauthorized_maps_from_transport() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/messages"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let base_url = server.uri();
        let err = tokio::task::spawn_blocking(move || {
            let provider = AnthropicProvider::with_base_url(base_url);
            provider.generate_with_key(&req(1), "bad-key").unwrap_err()
        })
        .await
        .unwrap();

        assert!(matches!(err, ProviderError::Unauthorized));
    }

    #[test]
    fn name_and_key_env_var_match_anthropic() {
        let provider = AnthropicProvider::new();
        assert_eq!(provider.name(), "anthropic");
        assert_eq!(provider.key_env_var(), Some("ANTHROPIC_API_KEY"));
    }
}
