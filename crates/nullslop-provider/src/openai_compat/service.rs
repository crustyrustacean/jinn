//! OpenAI-compatible streaming chat service.
//!
//! [`OpenAiCompatibleService`] implements [`LlmService`] using `reqwest`
//! to stream chat completions from any OpenAI-compatible provider.

use error_stack::{Report, ResultExt as _};
use futures::StreamExt as _;
use reqwest::Client;

use crate::llm_message::LlmMessage;
use crate::openai_compat::models;
use crate::openai_compat::provider_config::ProviderConfig;
use crate::openai_compat::request;
use crate::openai_compat::response::StreamResponseParser;
use crate::openai_compat::sse::{SseEvent, SseParser};
use crate::service::{ChatStream, LlmService, LlmServiceError, ToolStream};
use crate::stream_event::StreamEvent;
use crate::tool_types::ToolDefinition;

/// An LLM service that talks to an OpenAI-compatible API.
pub struct OpenAiCompatibleService {
    /// HTTP client (shared, connection-pooled).
    client: Client,
    /// Per-backend configuration.
    config: ProviderConfig,
    /// Model identifier.
    model: String,
    /// Base URL (override or default).
    base_url: String,
    /// API key for authentication.
    api_key: String,
    /// Extra body fields merged into every request.
    extra_body: serde_json::Map<String, serde_json::Value>,
}

impl OpenAiCompatibleService {
    /// Create a new service instance.
    #[must_use]
    pub fn new(
        client: Client,
        config: ProviderConfig,
        model: String,
        base_url: Option<String>,
        api_key: String,
        extra_body: Option<serde_json::Value>,
    ) -> Self {
        let base_url = base_url.unwrap_or_else(|| config.default_base_url.to_owned());
        let extra_body = match extra_body {
            Some(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        Self {
            client,
            config,
            model,
            base_url,
            api_key,
            extra_body,
        }
    }

    /// Fetch available model IDs from the provider.
    ///
    /// # Errors
    ///
    /// Returns [`LlmServiceError::Provider`] on HTTP or parse errors.
    pub async fn list_models(&self) -> Result<Vec<String>, Report<LlmServiceError>> {
        models::list_models(
            &self.client,
            &self.base_url,
            self.config.models_endpoint,
            &self.api_key,
            &self.config.custom_headers,
        )
        .await
    }

    /// Build and send a streaming chat completion request, returning the raw response.
    async fn send_streaming_request(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
    ) -> Result<reqwest::Response, Report<LlmServiceError>> {
        if self.api_key.is_empty() {
            return Err(Report::new(LlmServiceError::ApiKey)
                .attach(format!("Missing {} API key", self.config.name)));
        }

        let body =
            request::build_request(&self.model, messages, tools, &self.extra_body);

        let url = format!("{}{}", self.base_url, self.config.chat_endpoint);

        let mut req = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body);

        for (key, value) in &self.config.custom_headers {
            req = req.header(key.as_str(), value.as_str());
        }

        tracing::debug!("{} streaming request: POST {}", self.config.name, url);

        let response = req
            .send()
            .await
            .change_context(LlmServiceError::Provider)
            .attach(format!("{} streaming request failed", self.config.name))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "(no body)".to_owned());
            return Err(Report::new(LlmServiceError::Provider)
                .attach(format!("{} HTTP {status}", self.config.name))
                .attach(error_text));
        }

        Ok(response)
    }
}

fn create_tool_stream(
    response: reqwest::Response,
    provider_name: &str,
) -> ToolStream {
    let parser = StreamResponseParser::new();
    let sse = SseParser::new();

    let stream = response
        .bytes_stream()
        .scan((parser, sse, provider_name.to_owned()), |(parser, sse, name), chunk| {
            let results = match chunk {
                Ok(bytes) => {
                    let events = sse.feed(&bytes);
                    let mut stream_events = Vec::new();
                    for event in events {
                        match event {
                            SseEvent::Data(json) => {
                                stream_events.extend(
                                    parser.parse_data(&json)
                                        .into_iter()
                                        .map(Ok),
                                );
                            }
                            SseEvent::Done => {
                                stream_events.extend(
                                    parser.handle_done()
                                        .into_iter()
                                        .map(Ok),
                                );
                            }
                        }
                    }
                    stream_events
                }
                Err(e) => {
                    vec![Err(Report::new(LlmServiceError::Provider)
                        .attach(format!("{name} stream error"))
                        .attach(e.to_string()))]
                }
            };
            async move { Some(results) }
        })
        .flat_map(futures::stream::iter);

    Box::pin(stream)
}

#[async_trait::async_trait]
impl LlmService for OpenAiCompatibleService {
    async fn chat_stream(
        &self,
        messages: Vec<LlmMessage>,
    ) -> Result<ChatStream, Report<LlmServiceError>> {
        let response = self.send_streaming_request(&messages, &[]).await?;
        let parser = StreamResponseParser::new();
        let sse = SseParser::new();
        let name = self.config.name.to_owned();

        let stream = response
            .bytes_stream()
            .scan((parser, sse, name), |(parser, sse, name), chunk| {
                let results: Vec<Result<String, Report<LlmServiceError>>> = match chunk {
                    Ok(bytes) => {
                        let events = sse.feed(&bytes);
                        let mut tokens = Vec::new();
                        for event in events {
                            match event {
                                SseEvent::Data(json) => {
                                    for stream_event in parser.parse_data(&json) {
                                        match stream_event {
                                            StreamEvent::Text(text) => tokens.push(Ok(text)),
                                            StreamEvent::Done { .. } => {}
                                            _ => {} // Ignore reasoning/tools in text-only stream.
                                        }
                                    }
                                }
                                SseEvent::Done => {
                                    // Done is the stream terminator — no action needed for text-only.
                                }
                            }
                        }
                        tokens
                    }
                    Err(e) => {
                        vec![Err(Report::new(LlmServiceError::Provider)
                            .attach(format!("{name} stream error"))
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
        Ok(create_tool_stream(response, self.config.name))
    }
}

impl std::fmt::Debug for OpenAiCompatibleService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleService")
            .field("provider", &self.config.name)
            .field("model", &self.model)
            .finish()
    }
}
