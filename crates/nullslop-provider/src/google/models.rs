//! Model listing for Google Gemini.
//!
//! Fetches model IDs from Google's `models.list` endpoint.

use error_stack::{Report, ResultExt as _};
use reqwest::Client;

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
}

/// Fetch available model IDs from Google Gemini.
///
/// # Errors
///
/// Returns [`LlmServiceError::Provider`] on HTTP or parse errors.
pub async fn list_models(
    client: &Client,
    api_key: &str,
) -> Result<Vec<String>, Report<LlmServiceError>> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models?key={api_key}"
    );

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

    Ok(models.models.into_iter().map(|m| m.name).collect())
}
