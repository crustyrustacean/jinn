//! Factory for creating [`AnthropicService`] instances.

use error_stack::Report;
use reqwest::Client;

use crate::anthropic::service::AnthropicService;
use crate::service::{LlmService, LlmServiceError, LlmServiceFactory};
/// Factory for Anthropic LLM services.
#[derive(Debug)]
pub struct AnthropicFactory {
    /// Model identifier.
    model: String,
    /// API key for authentication.
    api_key: String,
    /// Shared HTTP client.
    client: Client,
    /// Display name for this factory instance.
    name: String,
}

impl AnthropicFactory {
    /// Create a new factory with an internally created HTTP client.
    #[must_use]
    pub fn new(model: String, api_key: String, name: String) -> Self {
        Self {
            model,
            api_key,
            client: Client::new(),
            name,
        }
    }

    /// Create a new factory with a shared HTTP client.
    #[must_use]
    pub fn with_client(client: Client, model: String, api_key: String, name: String) -> Self {
        Self {
            model,
            api_key,
            client,
            name,
        }
    }
}

impl LlmServiceFactory for AnthropicFactory {
    fn create(&self) -> Result<Box<dyn LlmService>, Report<LlmServiceError>> {
        if self.api_key.is_empty() {
            return Err(Report::new(LlmServiceError::ApiKey).attach("Missing Anthropic API key"));
        }

        let service = AnthropicService::with_client(
            self.client.clone(),
            self.model.clone(),
            self.api_key.clone(),
            None,
        );

        Ok(Box::new(service))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Clone for AnthropicFactory {
    fn clone(&self) -> Self {
        Self {
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            client: self.client.clone(),
            name: self.name.clone(),
        }
    }
}
