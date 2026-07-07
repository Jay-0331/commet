//! Ollama adapter — local models, no API key, and a request shape all
//! its own, so it shares nothing with [`super::openai_compat`].
//!
//! Like Anthropic, Ollama's `/api/chat` has no `n` parameter, so
//! `req.n` candidates come from repeated calls. Unlike Anthropic, the
//! target is a single local process that serves one model at a time —
//! flooding it with parallel requests just thrashes it, so we cap
//! in-flight calls at [`MAX_CONCURRENCY`].

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{GenerateRequest, HttpClient, Provider, ProviderError};

const DEFAULT_ENDPOINT: &str = "http://localhost:11434";

/// Most in-flight calls against the local Ollama process at once.
/// Higher just queues behind the single loaded model.
const MAX_CONCURRENCY: usize = 2;

/// A single message in the `/api/chat` `messages` array.
#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

/// Ollama sampling knobs. `num_predict` is Ollama's name for the
/// output-token cap.
#[derive(Debug, Serialize)]
struct Options {
    temperature: f32,
    num_predict: u32,
}

/// Request body for `POST {endpoint}/api/chat`. `stream: false` asks
/// for the whole message in one response instead of a token stream.
#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
    options: Options,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

pub struct OllamaProvider {
    client: HttpClient,
    endpoint: String,
}

impl OllamaProvider {
    pub fn new() -> Self {
        Self {
            // Local models can be slow to load; give them room.
            client: HttpClient::new(Duration::from_secs(120), 2),
            endpoint: DEFAULT_ENDPOINT.into(),
        }
    }

    #[cfg(test)]
    fn with_endpoint(endpoint: impl Into<String>) -> Self {
        Self {
            client: HttpClient::new(Duration::from_secs(5), 0),
            endpoint: endpoint.into(),
        }
    }

    /// Generate `req.n` candidates, at most [`MAX_CONCURRENCY`] calls
    /// in flight at a time. Candidates keep request order.
    fn generate_n(&self, req: &GenerateRequest) -> Result<Vec<String>, ProviderError> {
        let n = req.n.max(1) as usize;

        std::thread::scope(|scope| {
            let mut out = Vec::with_capacity(n);
            let mut remaining = n;

            while remaining > 0 {
                let batch = remaining.min(MAX_CONCURRENCY);
                let handles: Vec<_> = (0..batch)
                    .map(|_| scope.spawn(|| self.one_candidate(req)))
                    .collect();
                for handle in handles {
                    out.push(handle.join().expect("candidate request thread panicked")?);
                }
                remaining -= batch;
            }

            Ok(out)
        })
    }

    /// Issue one `/api/chat` call and return the message content.
    fn one_candidate(&self, req: &GenerateRequest) -> Result<String, ProviderError> {
        let url = format!("{}/api/chat", self.endpoint);
        let body = ChatRequest {
            model: req.model.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: req.system_prompt.clone(),
                },
                Message {
                    role: "user".into(),
                    content: req.user_prompt.clone(),
                },
            ],
            stream: false,
            options: Options {
                temperature: req.temperature,
                num_predict: req.max_tokens,
            },
        };

        // No auth header — Ollama is unauthenticated and local.
        let resp: ChatResponse = self.client.post_json(&url, &[], &body)?;
        Ok(resp.message.content)
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for OllamaProvider {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn key_env_var(&self) -> Option<&'static str> {
        None
    }

    fn generate(&self, req: &GenerateRequest) -> Result<Vec<String>, ProviderError> {
        self.generate_n(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn req(n: u8) -> GenerateRequest {
        GenerateRequest {
            system_prompt: "system instructions".into(),
            user_prompt: "diff text".into(),
            model: "llama3.1:8b".into(),
            max_tokens: 1024,
            temperature: 0.2,
            n,
        }
    }

    fn chat_body(content: &str) -> serde_json::Value {
        json!({"message": {"role": "assistant", "content": content}})
    }

    /// Records the peak number of overlapping requests so a test can
    /// assert the concurrency cap. Sleeps briefly so genuinely
    /// concurrent calls actually overlap in the window.
    struct ConcurrencyProbe {
        in_flight: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        body: serde_json::Value,
    }

    impl Respond for ConcurrencyProbe {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(self.body.clone())
        }
    }

    #[tokio::test]
    async fn single_call_posts_expected_url_and_body_without_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_body("feat: add x")))
            .mount(&server)
            .await;

        let endpoint = server.uri();
        let candidates = tokio::task::spawn_blocking(move || {
            let provider = OllamaProvider::with_endpoint(endpoint);
            provider.generate_n(&req(1)).unwrap()
        })
        .await
        .unwrap();

        assert_eq!(candidates, vec!["feat: add x".to_string()]);

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let request = &received[0];
        assert_eq!(request.url.path(), "/api/chat");
        assert!(request.headers.get("authorization").is_none());

        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "llama3.1:8b");
        assert_eq!(body["stream"], false);
        assert_eq!(body["options"]["temperature"], 0.2);
        assert_eq!(body["options"]["num_predict"], 1024);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "diff text");
    }

    #[tokio::test]
    async fn three_candidates_issue_three_calls_capped_at_two_concurrent() {
        let server = MockServer::start().await;
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ConcurrencyProbe {
                in_flight: in_flight.clone(),
                peak: peak.clone(),
                body: chat_body("feat: candidate"),
            })
            .mount(&server)
            .await;

        let endpoint = server.uri();
        let candidates = tokio::task::spawn_blocking(move || {
            let provider = OllamaProvider::with_endpoint(endpoint);
            provider.generate_n(&req(3)).unwrap()
        })
        .await
        .unwrap();

        assert_eq!(candidates.len(), 3);
        assert!(candidates.iter().all(|c| c == "feat: candidate"));

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 3);

        // Never more than MAX_CONCURRENCY in flight at once.
        assert!(
            peak.load(Ordering::SeqCst) <= MAX_CONCURRENCY,
            "peak concurrency {} exceeded cap {MAX_CONCURRENCY}",
            peak.load(Ordering::SeqCst),
        );
    }

    #[tokio::test]
    async fn transport_error_propagates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let endpoint = server.uri();
        let err = tokio::task::spawn_blocking(move || {
            let provider = OllamaProvider::with_endpoint(endpoint);
            provider.generate_n(&req(1)).unwrap_err()
        })
        .await
        .unwrap();

        assert!(matches!(err, ProviderError::BadResponse { .. }));
    }

    #[test]
    fn name_and_no_key_env_var() {
        let provider = OllamaProvider::new();
        assert_eq!(provider.name(), "ollama");
        assert_eq!(provider.key_env_var(), None);
    }
}
