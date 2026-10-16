//! Endpoint listing for OpenRouter models.
//!
//! GETs the per-model `/models/{id}/endpoints` endpoint and returns the
//! upstream **endpoints** (routing targets) OpenRouter can serve a model
//! through, each identified by its routing `tag`.
//!
//! Unlike the flat `/models` list, this response nests the endpoint array
//! under `data.endpoints` and each entry carries richer metadata (pricing,
//! uptime, quantization) used by the endpoint picker.

use error_stack::{Report, ResultExt as _};
use reqwest::Client;
use serde::Deserialize;

use crate::service::LlmServiceError;

/// Wrapper around the nested endpoint list returned by
/// `GET {base_url}/models/{id}/endpoints`.
///
/// OpenRouter nests the endpoints under `data.endpoints`, unlike the flat
/// `data` array of the `/models` endpoint.
#[derive(Debug, Deserialize)]
pub(crate) struct EndpointDetailsResponse {
    pub(crate) data: EndpointDetails,
}

/// The `data` object of the endpoint-details payload.
#[derive(Debug, Deserialize)]
pub(crate) struct EndpointDetails {
    /// The available upstream endpoints. Defaults to empty when missing.
    #[serde(default)]
    pub(crate) endpoints: Vec<RawEndpoint>,
}

/// A single raw upstream endpoint as returned by OpenRouter.
///
/// All fields beyond `provider_name` and `tag` are optional because providers
/// omit them inconsistently — never assume pricing or uptime are present.
/// The richer fields feed the endpoint picker's preview pane; until the
/// picker lands they are unused, hence the dead-code allow.
#[derive(Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "optional fields feed the not-yet-built picker preview"
)]
pub(crate) struct RawEndpoint {
    /// Human-readable upstream name (e.g. `"Anthropic"`). Display only.
    pub(crate) provider_name: String,
    /// The OpenRouter routing slug (e.g. `"anthropic"`). This is the value
    /// sent as `provider.order[0]` to force routing.
    pub(crate) tag: String,
    /// Quantization level reported by the upstream, if any.
    #[serde(default)]
    pub(crate) quantization: Option<String>,
    /// Max completion tokens the upstream supports, if reported (may be null).
    #[serde(default)]
    pub(crate) max_completion_tokens: Option<u32>,
    /// Parameters the upstream supports, if reported.
    #[serde(default)]
    pub(crate) supported_parameters: Vec<String>,
    /// Raw pricing object; shape varies and is only shown in the picker.
    #[serde(default)]
    pub(crate) pricing: Option<serde_json::Value>,
    /// 30-minute uptime percentage, if reported.
    #[serde(default)]
    pub(crate) uptime_last_30m: Option<f64>,
    /// 1-day uptime percentage, if reported.
    #[serde(default)]
    pub(crate) uptime_last_1d: Option<f64>,
    /// Median time-to-first-token over the last 30m, in ms. `null` without
    /// an `Authorization` header (the percentiles are auth-gated).
    #[serde(default)]
    pub(crate) latency_last_30m: Option<PercentileSet>,
    /// Median output throughput over the last 30m, in tok/s. `null` without
    /// an `Authorization` header.
    #[serde(default)]
    pub(crate) throughput_last_30m: Option<PercentileSet>,
    /// Conditional pricing rows (e.g. a `>200k` context tier), if any.
    #[serde(default)]
    pub(crate) overrides: Vec<PricingOverride>,
}

/// Auth-gated percentile statistics block (`{p50, p75, p90, p99}`).
///
/// Absent or `null` without an API key; only the median feeds the
/// market table's detail pane.
#[derive(Debug, Deserialize)]
#[allow(dead_code, reason = "only p50 feeds the market table today")]
pub(crate) struct PercentileSet {
    /// Median over the window.
    pub(crate) p50: Option<f64>,
    /// 75th percentile over the window.
    pub(crate) p75: Option<f64>,
    /// 90th percentile over the window.
    pub(crate) p90: Option<f64>,
    /// 99th percentile over the window.
    pub(crate) p99: Option<f64>,
}

/// One conditional pricing row: separate prices that apply only above
/// `min_prompt_tokens` prompt tokens (e.g. a `>200k` context tier).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PricingOverride {
    /// The prompt-token threshold the override prices kick in above;
    /// `None` when the upstream publishes the row without one.
    #[serde(default)]
    pub min_prompt_tokens: Option<u32>,
    /// Per-token prompt price under this override, if published.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Per-token completion price under this override, if published.
    #[serde(default)]
    pub completion: Option<String>,
    /// Per-token cached-input read price under this override, if published.
    #[serde(default)]
    pub input_cache_read: Option<String>,
}

/// A routing endpoint returned by [`list_endpoints`].
///
/// `tag` is the OpenRouter routing slug sent as `provider.order[0]` to force
/// routing to a single upstream; `provider_name` is the display label. The
/// remaining fields are optional metadata that feeds the endpoint picker's
/// preview pane (uptime, pricing, quantization) and the model-market slice's
/// detail pane (cache price, p50 latency/throughput, 1-day uptime, overrides).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EndpointInfo {
    /// The OpenRouter routing slug (e.g. `"anthropic"`).
    pub tag: String,
    /// Human-readable upstream name (e.g. `"Anthropic"`).
    pub provider_name: String,
    /// 30-minute uptime percentage, if reported.
    #[serde(default)]
    pub uptime_30m: Option<f64>,
    /// Per-token prompt price as reported by OpenRouter, if present.
    #[serde(default)]
    pub prompt_price: Option<String>,
    /// Per-token completion price as reported by OpenRouter, if present.
    #[serde(default)]
    pub completion_price: Option<String>,
    /// Quantization level reported by the upstream, if any.
    #[serde(default)]
    pub quantization: Option<String>,
    /// Max completion tokens the upstream supports, if reported.
    #[serde(default)]
    pub max_completion_tokens: Option<u32>,
    /// Per-token cached-input read price, if published.
    #[serde(default)]
    pub input_cache_read: Option<String>,
    /// 1-day uptime percentage, if reported.
    #[serde(default)]
    pub uptime_1d: Option<f64>,
    /// Median time-to-first-token over the last 30m, in ms. `None` without
    /// an API key (the percentiles are auth-gated) or when unreported.
    #[serde(default)]
    pub latency_p50_ms: Option<f64>,
    /// Median output throughput over the last 30m, in tok/s. `None` without
    /// an API key or when unreported.
    #[serde(default)]
    pub throughput_p50: Option<f64>,
    /// Conditional pricing rows; empty when the endpoint publishes none.
    #[serde(default)]
    pub overrides: Vec<PricingOverride>,
}

/// Fetch the routing endpoints available for a model on OpenRouter.
///
/// Sends `GET {base_url}/models/{model_id}/endpoints` with
/// `Authorization: Bearer {api_key}`. The `model_id` keeps its internal slash
/// (e.g. `anthropic/claude-sonnet-4.5`) — OpenRouter expects the literal id.
///
/// Results are sorted by `provider_name` for stable display ordering.
///
/// # Errors
///
/// Returns [`LlmServiceError::Provider`] on HTTP or parse errors.
pub async fn list_endpoints(
    client: &Client,
    base_url: &str,
    model_id: &str,
    api_key: &str,
    custom_headers: &[(String, String)],
) -> Result<Vec<EndpointInfo>, Report<LlmServiceError>> {
    let url = format!(
        "{}/models/{}/endpoints",
        base_url.trim_end_matches('/'),
        model_id
    );

    let mut request = client.get(&url).bearer_auth(api_key);

    for (key, value) in custom_headers {
        request = request.header(key.as_str(), value.as_str());
    }

    let response = request
        .send()
        .await
        .change_context(LlmServiceError::Provider)
        .attach("failed to send list_endpoints request")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "(no body)".to_owned());
        return Err(Report::new(LlmServiceError::Provider)
            .attach(format!("list_endpoints HTTP {status}"))
            .attach(error_text));
    }

    let body = response
        .text()
        .await
        .change_context(LlmServiceError::Provider)
        .attach("failed to read list_endpoints response body")?;

    let details: EndpointDetailsResponse = serde_json::from_str(&body)
        .change_context(LlmServiceError::Provider)
        .attach("failed to parse list_endpoints response")?;

    let mut endpoints: Vec<EndpointInfo> = details
        .data
        .endpoints
        .into_iter()
        .map(|raw| EndpointInfo {
            tag: raw.tag,
            provider_name: raw.provider_name,
            uptime_30m: raw.uptime_last_30m,
            prompt_price: raw
                .pricing
                .as_ref()
                .and_then(|p| p.get("prompt"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            completion_price: raw
                .pricing
                .as_ref()
                .and_then(|p| p.get("completion"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            quantization: raw.quantization,
            max_completion_tokens: raw.max_completion_tokens,
            input_cache_read: raw
                .pricing
                .as_ref()
                .and_then(|p| p.get("input_cache_read"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            uptime_1d: raw.uptime_last_1d,
            latency_p50_ms: raw.latency_last_30m.and_then(|l| l.p50),
            throughput_p50: raw.throughput_last_30m.and_then(|t| t.p50),
            overrides: raw.overrides,
        })
        .collect();
    endpoints.sort_by(|a, b| a.provider_name.cmp(&b.provider_name));
    Ok(endpoints)
}

/// Convenience wrapper that builds a default `reqwest::Client` before calling
/// [`list_endpoints`].
///
/// Use this when callers don't already hold a pooled client (e.g. the endpoint
/// picker, which fetches only when the picker is opened).
///
/// # Errors
///
/// Returns [`LlmServiceError::Provider`] if the client cannot be built or the
/// request/parse fails.
pub async fn list_endpoints_default_client(
    base_url: &str,
    model_id: &str,
    api_key: &str,
    custom_headers: &[(String, String)],
) -> Result<Vec<EndpointInfo>, Report<LlmServiceError>> {
    let client = Client::builder()
        .build()
        .change_context(LlmServiceError::Provider)
        .attach("failed to build reqwest client for list_endpoints")?;
    list_endpoints(&client, base_url, model_id, api_key, custom_headers).await
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_in_result,
        reason = "test code, panics are acceptable"
    )]
    use super::*;
    use reqwest::Client;

    #[rstest::rstest]
    #[tokio::test]
    async fn list_endpoints_returns_tag_and_provider_name_on_success() {
        // Given a mock server returning the nested /endpoints shape.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models/anthropic/claude-sonnet-4.5/endpoints")
            .match_header("authorization", "Bearer test-key")
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "data": {
                        "endpoints": [
                            {
                                "provider_name": "Bedrock",
                                "tag": "amazon-bedrock",
                                "uptime_last_30m": 99.9
                            },
                            {
                                "provider_name": "Anthropic",
                                "tag": "anthropic",
                                "uptime_last_30m": 99.7
                            }
                        ]
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();

        // When listing endpoints for the model.
        let result = list_endpoints(
            &client,
            &server.url(),
            "anthropic/claude-sonnet-4.5",
            "test-key",
            &[],
        )
        .await;

        // Then endpoints are returned, sorted by provider_name.
        let endpoints = result.expect("should succeed");
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].provider_name, "Anthropic");
        assert_eq!(endpoints[0].tag, "anthropic");
        assert_eq!(endpoints[1].provider_name, "Bedrock");
        assert_eq!(endpoints[1].tag, "amazon-bedrock");
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn list_endpoints_returns_error_on_http_failure() {
        // Given a mock server returning 404 for a nonexistent model.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models/no/such/model/endpoints")
            .with_status(404)
            .with_body("not found")
            .create_async()
            .await;

        let client = Client::new();

        // When listing endpoints for the nonexistent model.
        let result = list_endpoints(&client, &server.url(), "no/such/model", "test-key", &[]).await;

        // Then it returns an error (not Ok(vec![])).
        assert!(result.is_err(), "HTTP error should return Err, not Ok");
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn list_endpoints_tolerates_missing_optional_fields() {
        // Given a mock server returning endpoints with pricing null and no uptime.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models/m/test/endpoints")
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "data": {
                        "endpoints": [
                            {
                                "provider_name": "OpenAI",
                                "tag": "openai",
                                "pricing": null,
                                "max_completion_tokens": null
                            }
                        ]
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();

        // When listing endpoints for the model.
        let result = list_endpoints(&client, &server.url(), "m/test", "test-key", &[]).await;

        // Then it parses despite the missing/null optional fields.
        let endpoints = result.expect("should succeed despite missing optionals");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].tag, "openai");
        assert_eq!(endpoints[0].provider_name, "OpenAI");
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn list_endpoints_maps_cache_price_uptime_1d_p50s_and_overrides() {
        // Given a mock server returning the full auth'd endpoint shape:
        // cache pricing, 1-day uptime, the percentile blocks, and a
        // conditional pricing override.
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
                                "latency_last_30m": {"p50": 1159.0, "p75": 1400.0},
                                "throughput_last_30m": {"p50": 38.0, "p75": 50.0},
                                "overrides": [
                                    {
                                        "min_prompt_tokens": 200_000,
                                        "prompt": "0.000006",
                                        "completion": "0.00003",
                                        "input_cache_read": "0.0000006"
                                    }
                                ]
                            }
                        ]
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();

        // When listing endpoints for the model.
        let result = list_endpoints(&client, &server.url(), "anthropic/claude", "k", &[]).await;

        // Then every enriched field is mapped.
        let endpoints = result.expect("should succeed");
        assert_eq!(endpoints.len(), 1);
        let ep = &endpoints[0];
        assert_eq!(ep.input_cache_read.as_deref(), Some("0.0000003"));
        assert_eq!(ep.uptime_1d, Some(99.2));
        assert_eq!(ep.latency_p50_ms, Some(1159.0));
        assert_eq!(ep.throughput_p50, Some(38.0));
        assert_eq!(ep.overrides.len(), 1);
        assert_eq!(ep.overrides[0].min_prompt_tokens, Some(200_000));
        assert_eq!(ep.overrides[0].prompt.as_deref(), Some("0.000006"));
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn list_endpoints_keeps_p50s_none_when_percentiles_are_null() {
        // Given a mock server returning the unauthenticated shape: the
        // percentile blocks come back `null` without an API key.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models/m/test/endpoints")
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "data": {
                        "endpoints": [
                            {
                                "provider_name": "OpenAI",
                                "tag": "openai",
                                "latency_last_30m": null,
                                "throughput_last_30m": null
                            }
                        ]
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();

        // When listing endpoints without auth'd percentile data.
        let result = list_endpoints(&client, &server.url(), "m/test", "test-key", &[]).await;

        // Then the p50 fields stay `None` — the detail pane renders `—`.
        let endpoints = result.expect("should succeed");
        assert_eq!(endpoints[0].latency_p50_ms, None);
        assert_eq!(endpoints[0].throughput_p50, None);
        mock.assert_async().await;
    }
}
