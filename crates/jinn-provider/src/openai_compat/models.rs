//! Model listing for OpenAI-compatible providers.
//!
//! GETs the `/models` endpoint and returns model IDs. Used by
//! `DiscoverActor` for model discovery (critical for OpenRouter).

use error_stack::{Report, ResultExt as _};
use reqwest::Client;

use crate::InputModalities;
use crate::ModelInfo;
use crate::service::LlmServiceError;

/// Response from the OpenAI-compatible `/models` endpoint.
#[derive(Debug, serde::Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

/// A single model entry.
#[derive(Debug, serde::Deserialize)]
struct ModelEntry {
    id: String,
    /// Maximum context length in tokens (e.g. from OpenRouter's `context_length`).
    /// Not all providers return this field.
    context_length: Option<u32>,
}

/// Fetch available model IDs from an OpenAI-compatible provider.
///
/// Sends `GET {base_url}/models` with `Authorization: Bearer {api_key}`.
///
/// # Errors
///
/// Returns [`LlmServiceError::Provider`] on HTTP or parse errors.
pub async fn list_models(
    client: &Client,
    base_url: &str,
    models_endpoint: &str,
    api_key: &str,
    custom_headers: &[(String, String)],
) -> Result<Vec<ModelInfo>, Report<LlmServiceError>> {
    let url = format!("{}/{}", base_url.trim_end_matches('/'), models_endpoint);

    let mut request = client.get(&url).bearer_auth(api_key);

    for (key, value) in custom_headers {
        request = request.header(key.as_str(), value.as_str());
    }

    let response = request
        .send()
        .await
        .change_context(LlmServiceError::Provider)
        .attach("failed to send list_models request")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "(no body)".to_owned());
        return Err(Report::new(LlmServiceError::Provider)
            .attach(format!("list_models HTTP {status}"))
            .attach(error_text));
    }

    let body = response
        .text()
        .await
        .change_context(LlmServiceError::Provider)
        .attach("failed to read list_models response body")?;

    let models: ModelsResponse = serde_json::from_str(&body)
        .change_context(LlmServiceError::Provider)
        .attach("failed to parse list_models response")?;

    Ok(models
        .data
        .into_iter()
        .map(|m| ModelInfo {
            id: m.id,
            context_length: m.context_length,
            input_modalities: InputModalities::text(),
        })
        .collect())
}
