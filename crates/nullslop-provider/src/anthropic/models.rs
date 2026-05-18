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

/// Fetch available model IDs from Anthropic.
///
/// # Errors
///
/// Returns [`LlmServiceError::Provider`] on HTTP or parse errors.
pub async fn list_models(
    client: &Client,
    api_key: &str,
) -> Result<Vec<ModelInfo>, Report<LlmServiceError>> {
    let response = client
        .get("https://api.anthropic.com/v1/models")
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
