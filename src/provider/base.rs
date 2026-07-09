//! Shared HTTP transport for provider adapters.
//!
//! Centralizes `reqwest`, retry/backoff, and status→[`ProviderError`]
//! mapping so each adapter only has to build a request body and parse
//! a response — see the acceptance criteria on provider issue #28.

use std::thread;
use std::time::{Duration, SystemTime};

use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::ProviderError;

/// Header names whose values are never written to `tracing` output.
const REDACTED_HEADERS: &[&str] = &["authorization", "x-api-key"];

/// Longest response-body snippet kept in [`ProviderError::BadResponse`].
const SNIPPET_LIMIT: usize = 512;

/// Ceiling on the un-jittered backoff delay, in seconds. Attempt `n`
/// waits `min(2^n, 30)` seconds before the jitter multiplier.
const MAX_BACKOFF_SECS: u64 = 30;

/// Blocking HTTP client shared by every provider adapter.
///
/// Stateless aside from the underlying connection pool, so a single
/// instance can be reused across adapters and calls.
pub struct HttpClient {
    inner: Client,
    timeout: Duration,
    max_retries: u8,
}

impl HttpClient {
    /// Build a client with the given per-request timeout and retry
    /// budget for 429 / 5xx / transport failures.
    pub fn new(timeout: Duration, max_retries: u8) -> Self {
        let inner = Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest::blocking::Client::builder should never fail here");
        Self {
            inner,
            timeout,
            max_retries,
        }
    }

    /// POST `body` as JSON to `url` with `headers`, retrying on 429 /
    /// 5xx / transport errors, and deserialize the JSON response.
    pub fn post_json<Req, Resp>(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Req,
    ) -> Result<Resp, ProviderError>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        for attempt in 0..=self.max_retries {
            let mut builder = self.inner.post(url).json(body);
            for (name, value) in headers {
                builder = builder.header(*name, *value);
            }

            tracing::debug!(url, headers = ?redacted(headers), attempt, "provider request");

            let outcome = builder.send();
            let retries_left = attempt < self.max_retries;

            match outcome {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return resp.json::<Resp>().map_err(|e| ProviderError::BadResponse {
                            snippet: truncate(&e.to_string()),
                        });
                    }

                    if status == StatusCode::UNAUTHORIZED {
                        return Err(ProviderError::Unauthorized);
                    }

                    if status == StatusCode::TOO_MANY_REQUESTS {
                        let retry_after = retry_after(&resp);
                        if retries_left {
                            thread::sleep(retry_after.unwrap_or_else(|| backoff(attempt)));
                            continue;
                        }
                        return Err(ProviderError::RateLimited { retry_after });
                    }

                    if status.is_server_error() && retries_left {
                        thread::sleep(backoff(attempt));
                        continue;
                    }

                    let snippet = truncate(&resp.text().unwrap_or_default());
                    return Err(ProviderError::BadResponse { snippet });
                }
                Err(e) if e.is_timeout() => {
                    if retries_left {
                        thread::sleep(backoff(attempt));
                        continue;
                    }
                    return Err(ProviderError::Timeout);
                }
                Err(_) => {
                    if retries_left {
                        thread::sleep(backoff(attempt));
                        continue;
                    }
                    return Err(ProviderError::Network);
                }
            }
        }

        unreachable!("loop always returns on its final attempt")
    }

    /// Per-request timeout this client was built with.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Lightweight reachability probe: issue a `GET` and report whether
    /// the server produced *any* HTTP response. A non-2xx status (e.g.
    /// 401 from an unauthenticated `/models`) still counts as reachable
    /// — only a transport error or timeout means unreachable. Does not
    /// retry; the caller's timeout bounds the wait.
    pub fn reachable(&self, url: &str, headers: &[(&str, &str)]) -> bool {
        let mut builder = self.inner.get(url);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        tracing::debug!(url, headers = ?redacted(headers), "reachability probe");
        builder.send().is_ok()
    }
}

/// Parse a `Retry-After` header. Accepts either a whole-seconds count
/// (`Retry-After: 1`) or an HTTP-date (`Retry-After: Wed, 21 Oct 2015
/// 07:28:00 GMT`); a date already in the past yields `Duration::ZERO`.
fn retry_after(resp: &reqwest::blocking::Response) -> Option<Duration> {
    let value = resp.headers().get("retry-after")?.to_str().ok()?;
    let value = value.trim();

    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }

    let when = httpdate::parse_http_date(value).ok()?;
    Some(
        when.duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}

/// Exponential backoff for attempt `n` (0-indexed):
/// `min(2^n, MAX_BACKOFF_SECS) * (1 + jitter)` seconds, where `jitter`
/// is uniform in `[0, 0.5)`. Jitter spreads retries so a fleet of
/// clients hitting the same 429 doesn't stampede in lockstep.
fn backoff(attempt: u8) -> Duration {
    let base = 2u64.saturating_pow(attempt as u32).min(MAX_BACKOFF_SECS);
    Duration::from_secs_f64(base as f64 * (1.0 + jitter_fraction()))
}

/// A pseudo-random fraction in `[0, 0.5)`, seeded off the current
/// clock's sub-second nanos. Good enough to decorrelate retries
/// without pulling in an RNG dependency.
fn jitter_fraction() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 500) as f64 / 1000.0
}

/// Truncate a response body to [`SNIPPET_LIMIT`] bytes on a `char`
/// boundary so the snippet stays valid UTF-8.
fn truncate(body: &str) -> String {
    if body.len() <= SNIPPET_LIMIT {
        return body.to_string();
    }
    let mut end = SNIPPET_LIMIT;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    body[..end].to_string()
}

/// Redact sensitive header values before they reach `tracing` output.
fn redacted<'a>(headers: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    headers
        .iter()
        .map(|(name, value)| {
            if REDACTED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                (*name, "[REDACTED]")
            } else {
                (*name, *value)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[derive(Serialize)]
    struct Req {
        prompt: String,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Resp {
        message: String,
    }

    fn client() -> HttpClient {
        HttpClient::new(Duration::from_secs(5), 2)
    }

    fn body() -> Req {
        Req {
            prompt: "hi".into(),
        }
    }

    #[test]
    fn redacted_masks_authorization_and_x_api_key_case_insensitively() {
        let headers = [("Authorization", "Bearer secret"), ("X-API-Key", "k123")];
        let out = redacted(&headers);
        assert_eq!(
            out,
            vec![("Authorization", "[REDACTED]"), ("X-API-Key", "[REDACTED]")]
        );
    }

    #[test]
    fn redacted_leaves_other_headers_untouched() {
        let headers = [("Content-Type", "application/json")];
        assert_eq!(
            redacted(&headers),
            vec![("Content-Type", "application/json")]
        );
    }

    #[test]
    fn truncate_caps_long_body_at_snippet_limit() {
        let long = "x".repeat(SNIPPET_LIMIT + 100);
        assert_eq!(truncate(&long).len(), SNIPPET_LIMIT);
    }

    #[test]
    fn truncate_leaves_short_body_untouched() {
        assert_eq!(truncate("short"), "short");
    }

    #[tokio::test]
    async fn success_deserializes_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "ok"})))
            .mount(&server)
            .await;

        let url = format!("{}/chat", server.uri());
        let resp: Resp =
            tokio::task::spawn_blocking(move || client().post_json(&url, &[], &body()).unwrap())
                .await
                .unwrap();

        assert_eq!(
            resp,
            Resp {
                message: "ok".into()
            }
        );
    }

    #[tokio::test]
    async fn unauthorized_maps_to_unauthorized_without_retry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/chat", server.uri());
        let err = tokio::task::spawn_blocking(move || {
            client()
                .post_json::<_, Resp>(&url, &[], &body())
                .unwrap_err()
        })
        .await
        .unwrap();

        assert!(matches!(err, ProviderError::Unauthorized));
    }

    #[tokio::test]
    async fn rate_limited_retries_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "ok"})))
            .mount(&server)
            .await;

        let url = format!("{}/chat", server.uri());
        let resp: Resp =
            tokio::task::spawn_blocking(move || client().post_json(&url, &[], &body()).unwrap())
                .await
                .unwrap();

        assert_eq!(
            resp,
            Resp {
                message: "ok".into()
            }
        );
    }

    #[tokio::test]
    async fn honors_retry_after_seconds_before_retrying() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "1"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "ok"})))
            .mount(&server)
            .await;

        let url = format!("{}/chat", server.uri());
        let (resp, elapsed): (Resp, Duration) = tokio::task::spawn_blocking(move || {
            let start = std::time::Instant::now();
            let resp = client().post_json(&url, &[], &body()).unwrap();
            (resp, start.elapsed())
        })
        .await
        .unwrap();

        assert_eq!(
            resp,
            Resp {
                message: "ok".into()
            }
        );
        assert!(
            elapsed >= Duration::from_secs(1),
            "expected client to wait >= 1s for Retry-After, waited {elapsed:?}",
        );
    }

    #[tokio::test]
    async fn rate_limited_exhausted_returns_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
            .mount(&server)
            .await;

        let url = format!("{}/chat", server.uri());
        let err = tokio::task::spawn_blocking(move || {
            client()
                .post_json::<_, Resp>(&url, &[], &body())
                .unwrap_err()
        })
        .await
        .unwrap();

        match err {
            ProviderError::RateLimited { retry_after } => {
                assert_eq!(retry_after, Some(Duration::from_secs(0)));
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn malformed_json_maps_to_bad_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let url = format!("{}/chat", server.uri());
        let err = tokio::task::spawn_blocking(move || {
            client()
                .post_json::<_, Resp>(&url, &[], &body())
                .unwrap_err()
        })
        .await
        .unwrap();

        assert!(matches!(err, ProviderError::BadResponse { .. }));
    }

    #[tokio::test]
    async fn timeout_maps_to_timeout_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(200)))
            .mount(&server)
            .await;

        let url = format!("{}/chat", server.uri());
        let err = tokio::task::spawn_blocking(move || {
            let fast_client = HttpClient::new(Duration::from_millis(10), 0);
            fast_client
                .post_json::<_, Resp>(&url, &[], &body())
                .unwrap_err()
        })
        .await
        .unwrap();

        assert!(matches!(err, ProviderError::Timeout));
    }
}
