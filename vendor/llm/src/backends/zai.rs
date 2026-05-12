//! Z.ai API client implementation for chat functionality.
//!
//! This module provides integration with Z.ai's LLM models through their API.

use crate::{
    builder::LLMBackend,
    chat::{StructuredOutputFormat, Tool, ToolChoice},
    completion::{CompletionProvider, CompletionRequest, CompletionResponse},
    embedding::EmbeddingProvider,
    error::LLMError,
    models::{ModelListRequest, ModelListResponse, ModelsProvider, StandardModelListResponse},
    providers::openai_compatible::{OpenAICompatibleProvider, OpenAIProviderConfig},
    stt::SpeechToTextProvider,
    tts::TextToSpeechProvider,
    LLMProvider,
};
use async_trait::async_trait;

/// Z.ai configuration for the generic provider
pub struct ZaiConfig;

impl OpenAIProviderConfig for ZaiConfig {
    const PROVIDER_NAME: &'static str = "ZAI";
    const DEFAULT_BASE_URL: &'static str = "https://api.z.ai/api/coding/paas/v4/";
    const DEFAULT_MODEL: &'static str = "glm-5.1";
    const SUPPORTS_REASONING_EFFORT: bool = false;
    const SUPPORTS_STRUCTURED_OUTPUT: bool = false;
    const SUPPORTS_PARALLEL_TOOL_CALLS: bool = false;
    const SUPPORTS_STREAM_OPTIONS: bool = false;
}

pub type Zai = OpenAICompatibleProvider<ZaiConfig>;

impl Zai {
    /// Creates a new Z.ai client with the specified configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn with_config(
        api_key: impl Into<String>,
        base_url: Option<String>,
        model: Option<String>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
        timeout_seconds: Option<u64>,
        system: Option<String>,
        top_p: Option<f32>,
        top_k: Option<u32>,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
        extra_body: Option<serde_json::Value>,
        _embedding_encoding_format: Option<String>,
        _embedding_dimensions: Option<u32>,
        reasoning_effort: Option<String>,
        json_schema: Option<StructuredOutputFormat>,
        parallel_tool_calls: Option<bool>,
        normalize_response: Option<bool>,
    ) -> Self {
        OpenAICompatibleProvider::<ZaiConfig>::new(
            api_key,
            base_url,
            model,
            max_tokens,
            temperature,
            timeout_seconds,
            system,
            top_p,
            top_k,
            tools,
            tool_choice,
            reasoning_effort,
            json_schema,
            None, // voice - not supported by Z.ai
            extra_body,
            parallel_tool_calls,
            normalize_response,
            None, // embedding_encoding_format - not supported by Z.ai
            None, // embedding_dimensions - not supported by Z.ai
        )
    }
}

impl LLMProvider for Zai {
    fn tools(&self) -> Option<&[Tool]> {
        self.config.tools.as_deref()
    }
}

#[async_trait]
impl CompletionProvider for Zai {
    async fn complete(&self, _req: &CompletionRequest) -> Result<CompletionResponse, LLMError> {
        Ok(CompletionResponse {
            text: "Z.ai completion not implemented.".into(),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for Zai {
    async fn embed(&self, _text: Vec<String>) -> Result<Vec<Vec<f32>>, LLMError> {
        Err(LLMError::ProviderError(
            "Embedding not supported".to_string(),
        ))
    }
}

#[async_trait]
impl SpeechToTextProvider for Zai {
    async fn transcribe(&self, _audio: Vec<u8>) -> Result<String, LLMError> {
        Err(LLMError::ProviderError(
            "Z.ai does not implement speech to text endpoint yet.".into(),
        ))
    }
}

#[async_trait]
impl TextToSpeechProvider for Zai {}

#[async_trait]
impl ModelsProvider for Zai {
    async fn list_models(
        &self,
        _request: Option<&ModelListRequest>,
    ) -> Result<Box<dyn ModelListResponse>, LLMError> {
        if self.config.api_key.is_empty() {
            return Err(LLMError::AuthError("Missing Z.ai API key".to_string()));
        }

        let url = format!("{}/models", ZaiConfig::DEFAULT_BASE_URL);

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.config.api_key)
            .send()
            .await?
            .error_for_status()?;

        let result = StandardModelListResponse {
            inner: resp.json().await?,
            backend: LLMBackend::ZAI,
        };
        Ok(Box::new(result))
    }
}
