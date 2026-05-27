//! Model listing for Anthropic.
//!
//! Anthropic does have a `/v1/models` endpoint, but it requires the
//! `x-api-key` and `anthropic-version` headers.

use error_stack::{Report, ResultExt as _};
use reqwest::Client;

use crate::ModelInfo;
use crate::service::LlmServiceError;

/// Response from Anthropic's `/v1/models` endpoint.
#[derive(Debug, serde::Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

/// A single model entry.
#[derive(Debug, serde::Deserialize)]
struct ModelEntry {
    id: String,
    /// Maximum context window in tokens. Anthropic returns this as `context_window`.
    context_window: Option<u32>,
}

const MODELS_BASE_URL: &str = "https://api.anthropic.com";

/// Fetch available model IDs from Anthropic.
///
/// # Errors
///
/// Returns [`LlmServiceError::Provider`] on HTTP or parse errors.
pub async fn list_models(
    client: &Client,
    api_key: &str,
) -> Result<Vec<ModelInfo>, Report<LlmServiceError>> {
    list_models_with_base_url(client, api_key, MODELS_BASE_URL).await
}

/// Fetch models from a custom base URL (for testing).
pub(crate) async fn list_models_with_base_url(
    client: &Client,
    api_key: &str,
    base_url: &str,
) -> Result<Vec<ModelInfo>, Report<LlmServiceError>> {
    let url = format!("{base_url}/v1/models");
    let response = client
        .get(&url)
        .header("x-api-key", api_key)
        .header("content-type", "application/json")
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .change_context(LlmServiceError::Provider)
        .attach("failed to send Anthropic list_models request")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "(no body)".to_owned());
        return Err(Report::new(LlmServiceError::Provider)
            .attach(format!("Anthropic list_models HTTP {status}"))
            .attach(error_text));
    }

    let body = response
        .text()
        .await
        .change_context(LlmServiceError::Provider)
        .attach("failed to read Anthropic list_models response body")?;

    let models: ModelsResponse = serde_json::from_str(&body)
        .change_context(LlmServiceError::Provider)
        .attach("failed to parse Anthropic list_models response")?;

    Ok(models
        .data
        .into_iter()
        .map(|m| ModelInfo {
            id: m.id,
            context_length: m.context_window,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_in_result,
        reason = "test code, panics are acceptable"
    )]
    use crate::ModelInfo;

    use super::*;

    #[rstest::rstest]
    #[tokio::test]
    async fn list_models_returns_models_on_success() {
        // Given a mock server returning a valid models response.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/models")
            .match_header("x-api-key", "test-key")
            .with_status(200)
            .with_body(
                serde_json::json!({"data": [{"id": "claude-3", "context_window": 200000}]}).to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();
        let result = list_models_with_url(&client, "test-key", &server.url()).await;

        // Then models are returned.
        let models = result.expect("should succeed");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0], ModelInfo {
            id: "claude-3".to_owned(),
            context_length: Some(200_000),
        });
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn list_models_returns_error_on_http_failure() {
        // Given a mock server returning 403.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/models")
            .with_status(403)
            .with_body("forbidden")
            .create_async()
            .await;

        let client = Client::new();
        let result = list_models_with_url(&client, "bad-key", &server.url()).await;

        // Then it returns an error (not Ok(vec![])).
        assert!(result.is_err(), "HTTP error should return Err, not Ok");
        mock.assert_async().await;
    }

    /// Testable wrapper that delegates to the production function.
    async fn list_models_with_url(
        client: &Client,
        api_key: &str,
        base_url: &str,
    ) -> Result<Vec<ModelInfo>, Report<LlmServiceError>> {
        list_models_with_base_url(client, api_key, base_url).await
    }
}
