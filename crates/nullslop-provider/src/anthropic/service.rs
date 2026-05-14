//! Anthropic streaming chat service.
//!
//! [`AnthropicService`] implements [`LlmService`] using `reqwest`
//! to stream responses from Anthropic's Messages API.

use error_stack::{Report, ResultExt as _};
use futures::StreamExt as _;
use reqwest::Client;

use crate::anthropic::models;
use crate::anthropic::request;
use crate::anthropic::response::AnthropicStreamParser;
use crate::llm_message::LlmMessage;
use crate::openai_compat::sse::{SseEvent, SseParser};
use crate::service::{ChatStream, LlmService, LlmServiceError, ToolStream};
use crate::stream_event::StreamEvent;
use crate::tool_types::ToolDefinition;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1/messages";
const PROVIDER_NAME: &str = "Anthropic";

/// An LLM service that talks to Anthropic's Messages API.
pub struct AnthropicService {
    /// HTTP client.
    client: Client,
    /// Model identifier.
    model: String,
    /// API key for authentication.
    api_key: String,
    /// Optional system prompt override.
    system_prompt: Option<String>,
    /// Base URL override (for testing).
    base_url: String,
}

impl AnthropicService {
    /// Create a new Anthropic service instance.
    #[must_use]
    pub fn new(model: String, api_key: String, system_prompt: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            model,
            api_key,
            system_prompt,
            base_url: DEFAULT_BASE_URL.to_owned(),
        }
    }

    /// Create a new Anthropic service with a custom client.
    #[must_use]
    pub fn with_client(
        client: reqwest::Client,
        model: String,
        api_key: String,
        system_prompt: Option<String>,
    ) -> Self {
        Self {
            client,
            model,
            api_key,
            system_prompt,
            base_url: DEFAULT_BASE_URL.to_owned(),
        }
    }

    /// Create a new Anthropic service instance with a custom base URL (for testing).
    #[must_use]
    pub fn with_base_url(
        model: String,
        api_key: String,
        system_prompt: Option<String>,
        base_url: String,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            model,
            api_key,
            system_prompt,
            base_url,
        }
    }

    /// Fetch available model IDs from Anthropic.
    ///
    /// # Errors
    ///
    /// Returns [`LlmServiceError::Provider`] on HTTP or parse errors.
    pub async fn list_models(&self) -> Result<Vec<String>, Report<LlmServiceError>> {
        models::list_models(&self.client, &self.api_key).await
    }

    /// Send a streaming request to Anthropic's Messages API.
    async fn send_streaming_request(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
    ) -> Result<reqwest::Response, Report<LlmServiceError>> {
        if self.api_key.is_empty() {
            return Err(Report::new(LlmServiceError::ApiKey)
                .attach(format!("Missing {PROVIDER_NAME} API key")));
        }

        let body =
            request::build_request(&self.model, messages, tools, self.system_prompt.as_deref());

        let response = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("content-type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .change_context(LlmServiceError::Provider)
            .attach(format!("{PROVIDER_NAME} streaming request failed"))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "(no body)".to_owned());
            return Err(Report::new(LlmServiceError::Provider)
                .attach(format!("{PROVIDER_NAME} HTTP {status}"))
                .attach(error_text));
        }

        Ok(response)
    }
}

#[async_trait::async_trait]
impl LlmService for AnthropicService {
    async fn chat_stream(
        &self,
        messages: Vec<LlmMessage>,
    ) -> Result<ChatStream, Report<LlmServiceError>> {
        let response = self.send_streaming_request(&messages, &[]).await?;
        let parser = AnthropicStreamParser::new();
        let sse = SseParser::new();

        let stream = response
            .bytes_stream()
            .scan((parser, sse), |(parser, sse), chunk| {
                let results: Vec<Result<String, Report<LlmServiceError>>> = match chunk {
                    Ok(bytes) => {
                        let events = sse.feed(&bytes);
                        let mut tokens = Vec::new();
                        for event in events {
                            match event {
                                SseEvent::Data(json) => {
                                    if let Some(stream_event) = parser.parse_data(&json) {
                                        match stream_event {
                                            StreamEvent::Text(text) => tokens.push(Ok(text)),
                                            StreamEvent::Done { .. } => {}
                                            _ => {}
                                        }
                                    }
                                }
                                SseEvent::Done => {}
                            }
                        }
                        tokens
                    }
                    Err(e) => {
                        vec![Err(Report::new(LlmServiceError::Provider)
                            .attach(format!("{PROVIDER_NAME} stream error"))
                            .attach(e.to_string()))]
                    }
                };
                async move { Some(results) }
            })
            .flat_map(futures::stream::iter);

        Ok(Box::pin(stream))
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<LlmMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<ToolStream, Report<LlmServiceError>> {
        let response = self.send_streaming_request(&messages, &tools).await?;
        let parser = AnthropicStreamParser::new();
        let sse = SseParser::new();

        let stream = response
            .bytes_stream()
            .scan((parser, sse), |(parser, sse), chunk| {
                let results: Vec<Result<StreamEvent, Report<LlmServiceError>>> = match chunk {
                    Ok(bytes) => {
                        let events = sse.feed(&bytes);
                        let mut stream_events = Vec::new();
                        for event in events {
                            match event {
                                SseEvent::Data(json) => {
                                    if let Some(ev) = parser.parse_data(&json) {
                                        stream_events.push(Ok(ev));
                                    }
                                }
                                SseEvent::Done => {}
                            }
                        }
                        stream_events
                    }
                    Err(e) => {
                        vec![Err(Report::new(LlmServiceError::Provider)
                            .attach(format!("{PROVIDER_NAME} stream error"))
                            .attach(e.to_string()))]
                    }
                };
                async move { Some(results) }
            })
            .flat_map(futures::stream::iter);

        Ok(Box::pin(stream))
    }
}

impl std::fmt::Debug for AnthropicService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicService")
            .field("model", &self.model)
            .finish()
    }
}
