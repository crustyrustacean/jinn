//! Model listing for Google Gemini.
//!
//! Fetches model IDs from Google's `models.list` endpoint.

use error_stack::{Report, ResultExt as _};
use reqwest::Client;

use crate::ModelInfo;
use crate::service::LlmServiceError;

/// Response from Google's `/v1beta/models` endpoint.
#[derive(Debug, serde::Deserialize)]
struct ModelsResponse {
    models: Vec<ModelEntry>,
}

/// A single model entry.
#[derive(Debug, serde::Deserialize)]
struct ModelEntry {
    name: String,
    /// Maximum input token limit for this model.
    input_token_limit: Option<u32>,
}

/// Fetch available model IDs from Google Gemini.
///
/// # Errors
///
/// Returns [`LlmServiceError::Provider`] on HTTP or parse errors.
pub async fn list_models(
    client: &Client,
    api_key: &str,
) -> Result<Vec<ModelInfo>, Report<LlmServiceError>> {
    list_models_with_base_url(client, api_key, "https://generativelanguage.googleapis.com").await
}

/// Fetch models from a custom base URL (for testing).
pub(crate) async fn list_models_with_base_url(
    client: &Client,
    api_key: &str,
    base_url: &str,
) -> Result<Vec<ModelInfo>, Report<LlmServiceError>> {
    let url = format!("{base_url}/v1beta/models?key={api_key}");

    let response = client
        .get(&url)
        .send()
        .await
        .change_context(LlmServiceError::Provider)
        .attach("failed to send Google list_models request")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "(no body)".to_owned());
        return Err(Report::new(LlmServiceError::Provider)
            .attach(format!("Google list_models HTTP {status}"))
            .attach(error_text));
    }

    let body = response
        .text()
        .await
        .change_context(LlmServiceError::Provider)
        .attach("failed to read Google list_models response body")?;

    let models: ModelsResponse = serde_json::from_str(&body)
        .change_context(LlmServiceError::Provider)
        .attach("failed to parse Google list_models response")?;

    Ok(models
        .models
        .into_iter()
        .map(|m| ModelInfo {
            id: m.name,
            context_length: m.input_token_limit,
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
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1beta/models?key=test-key")
            .with_status(200)
            .with_body(
                serde_json::json!({"models": [{"name": "gemini-pro", "input_token_limit": 32000}]}).to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();
        let result = list_models_with_base_url(&client, "test-key", &server.url()).await;

        let models = result.expect("should succeed");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0], ModelInfo {
            id: "gemini-pro".to_owned(),
            context_length: Some(32_000),
        });
        mock.assert_async().await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn list_models_returns_error_on_http_failure() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1beta/models?key=bad-key")
            .with_status(403)
            .with_body("forbidden")
            .create_async()
            .await;

        let client = Client::new();
        let result = list_models_with_base_url(&client, "bad-key", &server.url()).await;

        assert!(result.is_err(), "HTTP error should return Err, not Ok");
        mock.assert_async().await;
    }
}
