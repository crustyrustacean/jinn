//! LM Studio API client using the OpenAI-compatible Chat Completions endpoint.
//!
//! LM Studio exposes `/v1/chat/completions` (and a limited `/v1/responses`).
//! Using the Chat Completions path avoids incompatibilities with the Responses
//! API's structured input format.

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

/// LM Studio configuration for the generic provider.
pub struct LmStudioConfig;

impl OpenAIProviderConfig for LmStudioConfig {
    const PROVIDER_NAME: &'static str = "LM Studio";
    const DEFAULT_BASE_URL: &'static str = "http://localhost:1234/v1/";
    const DEFAULT_MODEL: &'static str = "local-model";
}

/// LM Studio provider backed by the Chat Completions endpoint.
pub type LmStudio = OpenAICompatibleProvider<LmStudioConfig>;

impl LmStudio {
    /// Creates a new LM Studio client with the specified configuration.
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
        OpenAICompatibleProvider::<LmStudioConfig>::new(
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
            None, // voice
            extra_body,
            parallel_tool_calls,
            normalize_response,
            None, // embedding_encoding_format
            None, // embedding_dimensions
        )
    }
}

impl LLMProvider for LmStudio {
    fn tools(&self) -> Option<&[Tool]> {
        self.config.tools.as_deref()
    }
}

#[async_trait]
impl CompletionProvider for LmStudio {
    async fn complete(&self, _req: &CompletionRequest) -> Result<CompletionResponse, LLMError> {
        Ok(CompletionResponse {
            text: "LM Studio completion not implemented.".into(),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for LmStudio {
    async fn embed(&self, _text: Vec<String>) -> Result<Vec<Vec<f32>>, LLMError> {
        Err(LLMError::ProviderError(
            "Embedding not supported for LM Studio".to_string(),
        ))
    }
}

#[async_trait]
impl SpeechToTextProvider for LmStudio {
    async fn transcribe(&self, _audio: Vec<u8>) -> Result<String, LLMError> {
        Err(LLMError::ProviderError(
            "LM Studio does not implement speech to text.".into(),
        ))
    }
}

#[async_trait]
impl TextToSpeechProvider for LmStudio {}

#[async_trait]
impl ModelsProvider for LmStudio {
    async fn list_models(
        &self,
        _request: Option<&ModelListRequest>,
    ) -> Result<Box<dyn ModelListResponse>, LLMError> {
        let url = self
            .config
            .base_url
            .join("models")
            .map_err(|e| LLMError::HttpError(e.to_string()))?;

        let resp = self
            .client
            .get(url)
            .bearer_auth(&self.config.api_key)
            .send()
            .await?
            .error_for_status()?;

        let result = StandardModelListResponse {
            inner: resp.json().await?,
            backend: LLMBackend::LmStudio,
        };
        Ok(Box::new(result))
    }
}
