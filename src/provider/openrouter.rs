//! OpenRouter adapter — thin glue over [`super::openai_compat`].
//!
//! OpenRouter is OpenAI-compatible; the only differences from the
//! OpenAI adapter are the base URL, the model string being passed
//! through verbatim (e.g. `anthropic/claude-sonnet-4`), and two
//! optional attribution headers OpenRouter uses for its public
//! leaderboard (`HTTP-Referer`, `X-Title`).

use std::time::Duration;

use super::{ChatMessage, ChatRequest, GenerateRequest, HttpClient, Provider, ProviderError};

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_X_TITLE: &str = "commet";

pub struct OpenRouterProvider {
    client: HttpClient,
    base_url: String,
    http_referer: String,
    x_title: String,
}

impl OpenRouterProvider {
    pub fn new() -> Self {
        Self {
            client: HttpClient::new(Duration::from_secs(30), 2),
            base_url: DEFAULT_BASE_URL.into(),
            http_referer: String::new(),
            x_title: DEFAULT_X_TITLE.into(),
        }
    }

    #[cfg(test)]
    fn with_config(
        base_url: impl Into<String>,
        http_referer: impl Into<String>,
        x_title: impl Into<String>,
    ) -> Self {
        Self {
            client: HttpClient::new(Duration::from_secs(5), 0),
            base_url: base_url.into(),
            http_referer: http_referer.into(),
            x_title: x_title.into(),
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
        let mut headers = vec![("Authorization", auth.as_str())];
        if !self.http_referer.is_empty() {
            headers.push(("HTTP-Referer", self.http_referer.as_str()));
        }
        if !self.x_title.is_empty() {
            headers.push(("X-Title", self.x_title.as_str()));
        }

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

impl Default for OpenRouterProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for OpenRouterProvider {
    fn name(&self) -> &'static str {
        "openrouter"
    }

    fn key_env_var(&self) -> Option<&'static str> {
        Some("OPENROUTER_API_KEY")
    }

    fn generate(&self, req: &GenerateRequest) -> Result<Vec<String>, ProviderError> {
        let key = std::env::var("OPENROUTER_API_KEY").map_err(|_| ProviderError::MissingKey)?;
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
            model: "anthropic/claude-sonnet-4".into(),
            max_tokens: 1024,
            temperature: 0.2,
            n: 1,
        }
    }

    async fn mount_success(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "feat: add x"}}]
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn generate_posts_expected_url_auth_and_model_without_optional_headers() {
        let server = MockServer::start().await;
        mount_success(&server).await;

        let base_url = server.uri();
        let request = req();

        let candidates = tokio::task::spawn_blocking(move || {
            let provider = OpenRouterProvider::with_config(base_url, "", "");
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
        assert!(request.headers.get("http-referer").is_none());
        assert!(request.headers.get("x-title").is_none());

        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "anthropic/claude-sonnet-4");
    }

    #[tokio::test]
    async fn generate_includes_http_referer_and_x_title_when_set() {
        let server = MockServer::start().await;
        mount_success(&server).await;

        let base_url = server.uri();
        let request = req();

        tokio::task::spawn_blocking(move || {
            let provider =
                OpenRouterProvider::with_config(base_url, "https://example.com", "my-cli");
            provider.generate_with_key(&request, "test-key").unwrap()
        })
        .await
        .unwrap();

        let received = server.received_requests().await.unwrap();
        let request = &received[0];
        assert_eq!(
            request.headers.get("http-referer").unwrap(),
            "https://example.com"
        );
        assert_eq!(request.headers.get("x-title").unwrap(), "my-cli");
    }

    #[test]
    fn name_and_key_env_var_match_openrouter() {
        let provider = OpenRouterProvider::new();
        assert_eq!(provider.name(), "openrouter");
        assert_eq!(provider.key_env_var(), Some("OPENROUTER_API_KEY"));
    }
}
