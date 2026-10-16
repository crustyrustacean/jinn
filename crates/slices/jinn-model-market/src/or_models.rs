//! The OpenRouter models listing — fetch and parse of `GET /api/v1/models`.
//!
//! One call returns the whole catalog as [`RawModel`] listings. The
//! market's fields are: pricing (prompt/completion/`input_cache_read`,
//! plus the `overrides` conditional rows), `architecture.output_modalities`
//! (the text-output filter), `hugging_face_id` (the ECI join's fallback
//! key), `context_length`, and the id — which carries its variant suffix
//! (`:free`, `:thinking`) as part of the listing, so a variant is its own
//! row by construction.
//!
//! Parsing only: shaping into [`MarketRow`]s is [`crate::rows`]'s job,
//! and the actor owns the fetch/cache decision.

use error_stack::{Report, ResultExt};
use jinn_provider::PricingOverride;
use reqwest::Client;
use serde::Deserialize;
use wherror::Error;

/// A listing fetched from `/api/v1/models` failed to download or parse.
#[derive(Debug, Error)]
#[error(debug)]
pub struct OrModelsError;

/// The `/api/v1/models` response envelope.
#[derive(Debug, Deserialize)]
struct ModelsListResponse {
    /// Every listing OpenRouter publishes; `data` is always present.
    data: Vec<RawModel>,
}

/// One raw listing as OpenRouter returns it.
///
/// Fields the market needs are typed; everything else is ignored by
/// serde. The ids are free-form upstream strings and stay strings.
#[derive(Debug, Deserialize)]
pub struct RawModel {
    /// Full listing id (`author/slug` or `author/slug:variant`).
    pub id: String,
    /// Per-token pricing, when the listing publishes any.
    #[serde(default)]
    pub pricing: Option<RawPricing>,
    /// Architecture block; carries the output modalities.
    #[serde(default)]
    pub architecture: Option<RawArchitecture>,
    /// The Hugging Face model card id, for the ECI join's fallback key.
    #[serde(default)]
    pub hugging_face_id: Option<String>,
    /// Advertised context window, in tokens.
    #[serde(default)]
    pub context_length: Option<u32>,
}

/// The listing's `pricing` object.
#[derive(Debug, Deserialize)]
pub struct RawPricing {
    /// Per-token prompt price.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Per-token completion price.
    #[serde(default)]
    pub completion: Option<String>,
    /// Per-token cached-input read price; absent for listings that
    /// publish no cache price (decision F's `None`).
    #[serde(default)]
    pub input_cache_read: Option<String>,
    /// Conditional pricing rows (e.g. a `>200k` context tier).
    #[serde(default)]
    pub overrides: Vec<PricingOverride>,
}

/// The listing's `architecture` block.
#[derive(Debug, Deserialize)]
pub struct RawArchitecture {
    /// Output modalities; a listing without `"text"` is filtered out.
    #[serde(default)]
    pub output_modalities: Option<Vec<String>>,
}

/// Fetches and parses the models listing.
///
/// `api_key` is optional: `/models` works unauthenticated, so a missing
/// key must not gate the fetch — the listing simply omits the header.
///
/// # Errors
///
/// Returns [`OrModelsError`] on HTTP failure or a body that does not
/// parse; the report carries the status/body detail as attachments.
pub async fn fetch_models(
    client: &Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<RawModel>, Report<OrModelsError>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut request = client.get(&url);
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }

    let response = request
        .send()
        .await
        .change_context(OrModelsError)
        .attach("failed to send fetch_models request")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "(no body)".to_owned());
        return Err(Report::new(OrModelsError)
            .attach(format!("fetch_models HTTP {status}"))
            .attach(error_text));
    }

    let body = response
        .text()
        .await
        .change_context(OrModelsError)
        .attach("failed to read fetch_models response body")?;

    let parsed: ModelsListResponse = serde_json::from_str(&body)
        .change_context(OrModelsError)
        .attach("failed to parse fetch_models response")?;
    Ok(parsed.data)
}

/// The text-output gate: a listing is in the market when its
/// architecture publishes no modality list at all (conservatively text)
/// or a list containing `"text"`. ASR, image, and embedding listings
/// publish `["audio"]`/`["image"]`/`["embeddings"]` and drop out.
#[must_use]
pub fn is_text_output(model: &RawModel) -> bool {
    match &model.architecture {
        None => true,
        Some(arch) => match &arch.output_modalities {
            None => true,
            Some(modalities) => modalities.iter().any(|m| m == "text"),
        },
    }
}

/// Parses a per-token price string (`"0.000003"`, `"-1"`, `"0"`) into
/// USD/token.
///
/// OpenRouter publishes `"-1"` for unpriced/free entries on some
/// listings; that maps to `None` so the caller can decide (free
/// listings report their flag off pricing, not off this None).
#[must_use]
pub fn parse_price(raw: Option<&str>) -> Option<f64> {
    let text = raw?;
    let value: f64 = text.parse().ok()?;
    (value >= 0.0).then_some(value)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod or_models_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_in_result,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;

    /// A minimal well-formed `/models` body with one text listing and
    /// one audio (ASR) listing.
    fn catalog_body() -> serde_json::Value {
        serde_json::json!({
            "data": [
                {
                    "id": "anthropic/claude-sonnet-4.5",
                    "pricing": {
                        "prompt": "0.000003",
                        "completion": "0.000015",
                        "input_cache_read": "0.0000003",
                        "overrides": [
                            {
                                "min_prompt_tokens": 200_000,
                                "prompt": "0.000006",
                                "completion": "0.00003"
                            }
                        ]
                    },
                    "architecture": {
                        "output_modalities": ["text"]
                    },
                    "hugging_face_id": "anthropic/claude-sonnet-4.5",
                    "context_length": 200_000
                },
                {
                    "id": "openai/gpt-4o-audio-preview",
                    "pricing": {"prompt": "0.00001", "completion": "0.00002"},
                    "architecture": {
                        "output_modalities": ["audio"]
                    }
                },
                {
                    "id": "a/free-model:free",
                    "pricing": {"prompt": "0", "completion": "0"},
                    "context_length": 8192
                }
            ]
        })
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_models_parses_the_catalog() {
        // Given a mock server serving a catalog with pricing, modalities,
        // and overrides.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .with_status(200)
            .with_body(catalog_body().to_string())
            .create_async()
            .await;

        let client = Client::new();

        // When fetching the catalog unauthenticated (no key passed).
        let models = fetch_models(&client, &server.url(), None)
            .await
            .expect("fetch should succeed");

        // Then all three listings parse, with the market's fields.
        assert_eq!(models.len(), 3);
        let sonnet = &models[0];
        assert_eq!(sonnet.id, "anthropic/claude-sonnet-4.5");
        let pricing = sonnet.pricing.as_ref().expect("pricing present");
        assert_eq!(pricing.prompt.as_deref(), Some("0.000003"));
        assert_eq!(pricing.input_cache_read.as_deref(), Some("0.0000003"));
        assert_eq!(pricing.overrides.len(), 1);
        assert_eq!(pricing.overrides[0].min_prompt_tokens, Some(200_000));
        assert_eq!(sonnet.context_length, Some(200_000));
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_models_sends_the_bearer_header_only_with_a_key() {
        // Given a mock that requires the Authorization header.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .match_header("authorization", "Bearer k-1")
            .with_status(200)
            .with_body(catalog_body().to_string())
            .create_async()
            .await;

        let client = Client::new();

        // When fetching with a key.
        let models = fetch_models(&client, &server.url(), Some("k-1"))
            .await
            .expect("fetch should succeed");

        // Then the fetch succeeds and carries the header.
        assert!(!models.is_empty());
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_models_errors_on_http_failure() {
        // Given a mock returning 500.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .with_status(500)
            .with_body("boom")
            .create_async()
            .await;

        let client = Client::new();

        // When fetching.
        let result = fetch_models(&client, &server.url(), None).await;

        // Then it is an error carrying the status.
        let report = result.expect_err("HTTP failure must error");
        let rendered = format!("{report:?}");
        assert!(rendered.contains("500"), "{rendered}");
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[case(Some(&["text"][..]), true)]
    #[case(Some(&["text", "image"][..]), true)]
    #[case(Some(&["image"][..]), false)]
    #[case(Some(&["audio"][..]), false)]
    #[case(None, true)]
    fn is_text_output_filters_non_text_modalities(
        #[case] modalities: Option<&[&str]>,
        #[case] expected: bool,
    ) {
        // Given a listing with the modality set under test.
        let raw = serde_json::json!({
            "id": "a/m",
            "architecture": modalities.map(|ms| serde_json::json!({
                "output_modalities": ms
            }))
        });
        let model: RawModel = serde_json::from_value(raw).expect("fixture parses");

        // When gating on text output.
        let is_text = is_text_output(&model);

        // Then only text-publishing (or modality-silent) listings pass.
        assert_eq!(is_text, expected);
    }

    #[rstest::rstest]
    #[case(Some("0.000003"), Some(3e-6))]
    #[case(Some("0"), Some(0.0))]
    #[case(Some("-1"), None)]
    #[case(Some("free"), None)]
    #[case(None, None)]
    fn parse_price_maps_sentinels_to_none(
        #[case] raw: Option<&str>,
        #[case] expected: Option<f64>,
    ) {
        // Given a raw price string in OpenRouter's format.
        // When parsing it.
        let parsed = parse_price(raw);

        // Then "-1" and junk map to None; valid strings parse exactly.
        match (parsed, expected) {
            (Some(got), Some(want)) => assert!((got - want).abs() < 1e-12),
            (None, None) => {}
            (parsed, expected) => panic!("parsed {parsed:?} but expected {expected:?}"),
        }
    }
}
