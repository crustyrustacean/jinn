//! Per-model endpoint detail — the market's slice of
//! `GET /api/v1/models/{id}/endpoints`.
//!
//! A thin adapter over [`jinn_provider::list_endpoints`]: that function
//! owns the request, the auth header, and the (extended)
//! [`jinn_provider::EndpointInfo`] mapping; this module re-exports the
//! result under a market-local name so the actor reads one vocabulary
//! and the `-msg` row conversion lives beside it.
//!
//! Percentile p50s come back `None` without an API key (research §2.4):
//! the detail pane renders `—`, and nothing here treats that as an
//! error.

use error_stack::Report;
use jinn_provider::LlmServiceError;

/// The market's endpoint-detail row: the provider crate's enriched
/// endpoint, re-exported.
pub use jinn_provider::EndpointInfo as MarketEndpointInfo;

/// Fetches one model's endpoint detail.
///
/// The `model_id` is the *OpenRouter* listing id (`author/slug[:variant]`,
/// no `openrouter/` prefix). `api_key` rides as the bearer header when
/// present; the percentiles are auth-gated upstream and arrive `None`
/// without it.
///
/// # Errors
///
/// Returns the provider crate's [`LlmServiceError::Provider`] report on
/// HTTP or parse failure.
pub async fn fetch_endpoints(
    client: &reqwest::Client,
    base_url: &str,
    model_id: &str,
    api_key: Option<&str>,
) -> Result<Vec<MarketEndpointInfo>, Report<LlmServiceError>> {
    jinn_provider::list_endpoints(client, base_url, model_id, api_key.unwrap_or_default(), &[])
        .await
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod or_endpoints_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_in_result,
        reason = "test code"
    )]

    use super::*;

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_endpoints_returns_the_market_endpoint_shape() {
        // Given a mock server serving one auth'd endpoint with the
        // market's fields.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models/anthropic/claude/endpoints")
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "data": {
                        "endpoints": [
                            {
                                "provider_name": "Anthropic",
                                "tag": "anthropic",
                                "pricing": {
                                    "prompt": "0.000003",
                                    "completion": "0.000015",
                                    "input_cache_read": "0.0000003"
                                },
                                "uptime_last_1d": 99.2,
                                "latency_last_30m": {"p50": 1159.0},
                                "throughput_last_30m": {"p50": 38.0}
                            }
                        ]
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();

        // When fetching the model's endpoints through the market adapter.
        let endpoints = fetch_endpoints(&client, &server.url(), "anthropic/claude", Some("k"))
            .await
            .expect("fetch should succeed");

        // Then the market fields ride through.
        let (tag, cache_read, latency, uptime) = {
            let ep = endpoints.first().expect("one endpoint");
            (
                ep.tag.clone(),
                ep.input_cache_read.clone(),
                ep.latency_p50_ms,
                ep.uptime_1d,
            )
        };
        assert_eq!(tag, "anthropic");
        assert_eq!(cache_read.as_deref(), Some("0.0000003"));
        assert_eq!(latency, Some(1159.0));
        assert_eq!(uptime, Some(99.2));
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_endpoints_propagates_http_errors() {
        // Given a mock returning 404 for a nonexistent model.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models/no/such/endpoints")
            .with_status(404)
            .with_body("not found")
            .create_async()
            .await;

        let client = reqwest::Client::new();

        // When fetching.
        let result = fetch_endpoints(&client, &server.url(), "no/such", None).await;

        // Then the error propagates as the provider report.
        assert!(result.is_err(), "HTTP failure must error");
        mock.assert_async().await;
    }
}
