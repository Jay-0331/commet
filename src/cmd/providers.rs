//! `cc providers` — a key + reachability matrix for the four builtin
//! backends, so users can see what's usable before they hit a
//! confusing generate-time error.
//!
//! Each row reports the configured model, whether the provider's API
//! key is present in the environment (Ollama needs none), and whether
//! the provider's listing endpoint (`/models`, or `/api/tags` for
//! Ollama) answered a lightweight `GET` within a 5 s timeout.

use std::io::IsTerminal;
use std::time::Duration;

use serde::Serialize;

use crate::cli::ProvidersArgs;
use crate::config::{ColorMode, Config};
use crate::error::Result;
use crate::provider::HttpClient;

/// Reachability probes get their own short timeout, independent of the
/// per-provider generate timeout.
const REACHABILITY_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a provider's API key was found in the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum KeyState {
    Present,
    Missing,
    /// Provider needs no key (local Ollama).
    NotRequired,
}

/// One row of the status matrix.
#[derive(Debug, Serialize)]
struct ProviderStatus {
    provider: &'static str,
    model: String,
    key: KeyState,
    reachable: bool,
}

/// A provider's reachability target, resolved from config.
struct Probe {
    provider: &'static str,
    model: String,
    key_env: Option<&'static str>,
    url: String,
    headers: Vec<(&'static str, String)>,
}

/// Build the probe list from the effective config. Endpoints for
/// Ollama/OpenRouter come from config (they're user-configurable);
/// Anthropic/OpenAI are fixed hosts.
fn probes(config: &Config) -> Vec<Probe> {
    let p = &config.providers;
    vec![
        Probe {
            provider: "anthropic",
            model: p.anthropic.model.clone(),
            key_env: Some("ANTHROPIC_API_KEY"),
            url: "https://api.anthropic.com/v1/models".into(),
            headers: vec![("anthropic-version", "2023-06-01".into())],
        },
        Probe {
            provider: "openai",
            model: p.openai.model.clone(),
            key_env: Some("OPENAI_API_KEY"),
            url: "https://api.openai.com/v1/models".into(),
            headers: vec![],
        },
        Probe {
            provider: "openrouter",
            model: p.openrouter.model.clone(),
            key_env: Some("OPENROUTER_API_KEY"),
            url: format!("{}/models", p.openrouter.endpoint),
            headers: vec![],
        },
        Probe {
            provider: "ollama",
            model: p.ollama.model.clone(),
            key_env: None,
            url: format!("{}/api/tags", p.ollama.endpoint),
            headers: vec![],
        },
    ]
}

/// Look up whether `key_env` names a non-empty environment variable.
fn key_state(key_env: Option<&str>) -> KeyState {
    match key_env {
        None => KeyState::NotRequired,
        Some(var) => match std::env::var(var) {
            Ok(v) if !v.is_empty() => KeyState::Present,
            _ => KeyState::Missing,
        },
    }
}

/// Evaluate one probe: environment key check + reachability `GET`.
fn evaluate(client: &HttpClient, probe: &Probe) -> ProviderStatus {
    let header_refs: Vec<(&str, &str)> = probe
        .headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    ProviderStatus {
        provider: probe.provider,
        model: probe.model.clone(),
        key: key_state(probe.key_env),
        reachable: client.reachable(&probe.url, &header_refs),
    }
}

/// Should output carry ANSI color? `Always`/`Never` are absolute;
/// `Auto` colors only when stdout is a terminal.
fn use_color(mode: ColorMode) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => std::io::stdout().is_terminal(),
    }
}

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn paint(text: &str, color: &str, enabled: bool) -> String {
    if enabled {
        format!("{color}{text}{RESET}")
    } else {
        text.to_string()
    }
}

fn key_cell(key: KeyState) -> (&'static str, &'static str) {
    match key {
        KeyState::Present => ("yes", GREEN),
        KeyState::Missing => ("no", RED),
        KeyState::NotRequired => ("n/a", DIM),
    }
}

/// Render the matrix as a fixed-width table.
fn render_table(rows: &[ProviderStatus], color: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<12} {:<28} {:<5} {}\n",
        "PROVIDER", "MODEL", "KEY", "REACHABLE"
    ));
    for row in rows {
        let (key_text, key_color) = key_cell(row.key);
        let (reach_text, reach_color) = if row.reachable {
            ("yes", GREEN)
        } else {
            ("no", RED)
        };
        // Pad before painting so ANSI codes don't skew column widths.
        out.push_str(&format!(
            "{:<12} {:<28} {} {}\n",
            row.provider,
            row.model,
            paint(&format!("{key_text:<5}"), key_color, color),
            paint(reach_text, reach_color, color),
        ));
    }
    out
}

fn render_json(rows: &[ProviderStatus]) -> String {
    serde_json::to_string_pretty(rows).expect("provider status rows serialize to JSON")
}

/// Run `cc providers`: probe all four backends and print the matrix.
pub fn run(config: &Config, args: &ProvidersArgs) -> Result<()> {
    let client = HttpClient::new(REACHABILITY_TIMEOUT, 0);
    let rows: Vec<ProviderStatus> = probes(config)
        .iter()
        .map(|probe| evaluate(&client, probe))
        .collect();

    let out = if args.json {
        render_json(&rows)
    } else {
        render_table(&rows, use_color(config.ui.color))
    };

    print!("{out}");
    if !out.ends_with('\n') {
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn probe(provider: &'static str, key_env: Option<&'static str>, url: String) -> Probe {
        Probe {
            provider,
            model: "some-model".into(),
            key_env,
            url,
            headers: vec![],
        }
    }

    #[test]
    fn key_state_maps_env_presence() {
        // Unique names so we never collide with real provider vars or
        // other tests running in parallel.
        assert_eq!(key_state(None), KeyState::NotRequired);
        assert_eq!(
            key_state(Some("CC_PROVIDERS_TEST_DEFINITELY_UNSET")),
            KeyState::Missing
        );

        // SAFETY: single-threaded within this test; the var name is
        // unique to this test so no other thread reads or writes it.
        unsafe { std::env::set_var("CC_PROVIDERS_TEST_PRESENT", "sk-xxx") };
        assert_eq!(
            key_state(Some("CC_PROVIDERS_TEST_PRESENT")),
            KeyState::Present
        );
        unsafe { std::env::set_var("CC_PROVIDERS_TEST_EMPTY", "") };
        assert_eq!(
            key_state(Some("CC_PROVIDERS_TEST_EMPTY")),
            KeyState::Missing
        );
        unsafe {
            std::env::remove_var("CC_PROVIDERS_TEST_PRESENT");
            std::env::remove_var("CC_PROVIDERS_TEST_EMPTY");
        }
    }

    /// All four status combinations: {key present, key missing} ×
    /// {reachable, unreachable}, evaluated against a real reachability
    /// probe (wiremock live vs. a dead address).
    #[tokio::test]
    async fn evaluate_covers_all_four_combinations() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .mount(&server)
            .await;

        let live = server.uri();
        // Reserved-for-documentation address that refuses fast.
        let dead = "http://127.0.0.1:1".to_string();

        let statuses = tokio::task::spawn_blocking(move || {
            // SAFETY: unique var names, mutated only here.
            unsafe { std::env::set_var("CC_PROVIDERS_COMBO_PRESENT", "sk-xxx") };
            let client = HttpClient::new(Duration::from_secs(5), 0);

            let present_reachable = evaluate(
                &client,
                &probe("p1", Some("CC_PROVIDERS_COMBO_PRESENT"), live.clone()),
            );
            let present_unreachable = evaluate(
                &client,
                &probe("p2", Some("CC_PROVIDERS_COMBO_PRESENT"), dead.clone()),
            );
            let missing_reachable = evaluate(
                &client,
                &probe("p3", Some("CC_PROVIDERS_COMBO_UNSET"), live),
            );
            let missing_unreachable = evaluate(
                &client,
                &probe("p4", Some("CC_PROVIDERS_COMBO_UNSET"), dead),
            );

            unsafe { std::env::remove_var("CC_PROVIDERS_COMBO_PRESENT") };
            [
                present_reachable,
                present_unreachable,
                missing_reachable,
                missing_unreachable,
            ]
        })
        .await
        .unwrap();

        assert_eq!(statuses[0].key, KeyState::Present);
        assert!(statuses[0].reachable);
        assert_eq!(statuses[1].key, KeyState::Present);
        assert!(!statuses[1].reachable);
        assert_eq!(statuses[2].key, KeyState::Missing);
        assert!(statuses[2].reachable);
        assert_eq!(statuses[3].key, KeyState::Missing);
        assert!(!statuses[3].reachable);
    }

    #[test]
    fn probes_use_config_endpoints_and_models() {
        let mut config = Config::default();
        config.providers.ollama.endpoint = "http://box:9999".into();
        config.providers.openrouter.endpoint = "https://router.test/api".into();

        let probes = probes(&config);
        let by = |name: &str| probes.iter().find(|p| p.provider == name).unwrap();

        assert_eq!(by("ollama").url, "http://box:9999/api/tags");
        assert_eq!(by("ollama").key_env, None);
        assert_eq!(by("openrouter").url, "https://router.test/api/models");
        assert_eq!(by("anthropic").url, "https://api.anthropic.com/v1/models");
        assert_eq!(by("openai").url, "https://api.openai.com/v1/models");
        assert_eq!(by("anthropic").model, config.providers.anthropic.model);
    }

    fn rows() -> Vec<ProviderStatus> {
        vec![
            ProviderStatus {
                provider: "anthropic",
                model: "claude-sonnet-4-6".into(),
                key: KeyState::Present,
                reachable: true,
            },
            ProviderStatus {
                provider: "ollama",
                model: "llama3.1:8b".into(),
                key: KeyState::NotRequired,
                reachable: false,
            },
        ]
    }

    #[test]
    fn table_without_color_has_no_ansi_codes() {
        let table = render_table(&rows(), false);
        assert!(!table.contains('\x1b'));
        assert!(table.contains("anthropic"));
        assert!(table.contains("claude-sonnet-4-6"));
        assert!(table.contains("n/a")); // ollama needs no key
    }

    #[test]
    fn table_with_color_emits_ansi_codes() {
        let table = render_table(&rows(), true);
        assert!(table.contains(GREEN));
        assert!(table.contains(RED));
    }

    #[test]
    fn json_output_is_parseable_and_keyed() {
        let parsed: serde_json::Value = serde_json::from_str(&render_json(&rows())).unwrap();
        assert_eq!(parsed[0]["provider"], "anthropic");
        assert_eq!(parsed[0]["key"], "present");
        assert_eq!(parsed[0]["reachable"], true);
        assert_eq!(parsed[1]["key"], "not_required");
    }

    #[test]
    fn use_color_respects_always_and_never() {
        assert!(use_color(ColorMode::Always));
        assert!(!use_color(ColorMode::Never));
    }
}
