//! Shared wire types for OpenAI-compatible chat completion APIs.
//!
//! OpenAI and OpenRouter speak the same dialect (`POST
//! {base_url}/chat/completions`), so both adapters build a
//! [`ChatRequest`] and call [`complete`] instead of duplicating the
//! request/response shape and the empty-choices edge case.

use serde::{Deserialize, Serialize};

use super::{HttpClient, ProviderError};

/// A single message in the `messages` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Request body for `POST {base_url}/chat/completions`.
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub n: u8,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChatMessage,
}

/// Issue one HTTP call requesting `n` candidates and return their
/// message contents. `req.n` is overridden with `n` so callers can
/// build a template request once and vary the candidate count per
/// call. An empty `choices` array is treated as [`ProviderError::BadResponse`].
pub fn complete(
    client: &HttpClient,
    base_url: &str,
    headers: &[(&str, &str)],
    req: &ChatRequest,
    n: u8,
) -> Result<Vec<String>, ProviderError> {
    let url = format!("{base_url}/chat/completions");
    let body = ChatRequest { n, ..req.clone() };

    let resp: ChatResponse = client.post_json(&url, headers, &body)?;

    if resp.choices.is_empty() {
        return Err(ProviderError::BadResponse {
            snippet: "response contained no choices".into(),
        });
    }

    Ok(resp
        .choices
        .into_iter()
        .map(|choice| choice.message.content)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn req() -> ChatRequest {
        ChatRequest {
            model: "gpt-test".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "diff".into(),
            }],
            n: 1,
            max_tokens: 256,
            temperature: 0.7,
        }
    }

    fn choices_body(contents: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "choices": contents
                .iter()
                .map(|c| serde_json::json!({"message": {"role": "assistant", "content": c}}))
                .collect::<Vec<_>>()
        })
    }

    #[tokio::test]
    async fn single_candidate_round_trips() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(choices_body(&["feat: add x"])))
            .mount(&server)
            .await;

        let base_url = server.uri();
        let candidates = tokio::task::spawn_blocking(move || {
            let client = HttpClient::new(Duration::from_secs(5), 0);
            complete(&client, &base_url, &[], &req(), 1).unwrap()
        })
        .await
        .unwrap();

        assert_eq!(candidates, vec!["feat: add x".to_string()]);
    }

    #[tokio::test]
    async fn three_candidates_round_trip_in_order() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(choices_body(&["feat: a", "feat: b", "feat: c"])),
            )
            .mount(&server)
            .await;

        let base_url = server.uri();
        let candidates = tokio::task::spawn_blocking(move || {
            let client = HttpClient::new(Duration::from_secs(5), 0);
            complete(&client, &base_url, &[], &req(), 3).unwrap()
        })
        .await
        .unwrap();

        assert_eq!(
            candidates,
            vec![
                "feat: a".to_string(),
                "feat: b".to_string(),
                "feat: c".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn empty_choices_surfaces_as_bad_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(choices_body(&[])))
            .mount(&server)
            .await;

        let base_url = server.uri();
        let err = tokio::task::spawn_blocking(move || {
            let client = HttpClient::new(Duration::from_secs(5), 0);
            complete(&client, &base_url, &[], &req(), 1).unwrap_err()
        })
        .await
        .unwrap();

        assert!(matches!(err, ProviderError::BadResponse { .. }));
    }
}
