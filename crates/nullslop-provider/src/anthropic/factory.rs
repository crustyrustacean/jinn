//! Factory for creating [`AnthropicService`] instances.

use error_stack::Report;
use reqwest::Client;

use crate::anthropic::service::AnthropicService;
use crate::service::{LlmService, LlmServiceFactory, LlmServiceError};

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
    /// Create a new factory.
    #[must_use]
    pub fn new(model: String, api_key: String, client: Client, name: String) -> Self {
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
            return Err(Report::new(LlmServiceError::ApiKey)
                .attach("Missing Anthropic API key"));
        }

        let service =
            AnthropicService::new(self.client.clone(), self.model.clone(), self.api_key.clone(), None);

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
