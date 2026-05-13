//! Resolved provider — a single model expanded from a provider block.

use super::provider_id::ProviderId;

/// A fully resolved provider entry — one per model.
///
/// Created by expanding each [`ProviderEntry`](super::config::ProviderEntry)'s
/// `models` list, or by merging runtime-discovered models from the model cache.
/// This is the internal representation used by the registry for lookup,
/// availability checks, and factory creation.
#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    /// Unique ID in `{name}/{model}` format (e.g., `"ollama/llama3"`).
    pub id: ProviderId,
    /// Provider block name (e.g., `"ollama"`).
    pub name: String,
    /// Model identifier (e.g., `"llama3"`).
    pub model: String,
    /// Backend type string.
    pub backend: String,
    /// Optional base URL override.
    pub base_url: Option<String>,
    /// Environment variable name holding the API key.
    pub api_key_env: Option<String>,
    /// Whether this provider requires an API key.
    pub requires_key: bool,
    /// Extra JSON body parameters for the LLM builder.
    pub extra_body: Option<serde_json::Value>,
    /// Whether this model was discovered from a remote provider at runtime
    /// (not statically listed in `providers.toml`).
    pub is_remote: bool,
}
