//! Factory for creating [`GoogleService`] instances.

use error_stack::Report;
use reqwest::Client;

use crate::google::service::GoogleService;
use crate::service::{LlmService, LlmServiceFactory, LlmServiceError};

/// Factory for Google Gemini LLM services.
#[derive(Debug)]
pub struct GoogleFactory {
    /// Model identifier.
    model: String,
    /// API key for authentication.
    api_key: String,
    /// Shared HTTP client.
    client: Client,
    /// Display name.
    name: String,
}

impl GoogleFactory {
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

impl LlmServiceFactory for GoogleFactory {
    fn create(&self) -> Result<Box<dyn LlmService>, Report<LlmServiceError>> {
        if self.api_key.is_empty() {
            return Err(Report::new(LlmServiceError::ApiKey)
                .attach("Missing Google API key"));
        }

        let service =
            GoogleService::new(self.client.clone(), self.model.clone(), self.api_key.clone());

        Ok(Box::new(service))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Clone for GoogleFactory {
    fn clone(&self) -> Self {
        Self {
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            client: self.client.clone(),
            name: self.name.clone(),
        }
    }
}
