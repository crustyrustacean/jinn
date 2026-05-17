//! LLM provider abstraction — streaming chat completions and model discovery.
//!
//! Defines the [`LlmService`] trait for streaming LLM responses and
//! [`LlmServiceFactory`] for creating per-call service instances.
//! Includes test doubles (fake, sample, no-providers) for use in tests.

mod backend;
mod fake;
mod llm_message;
mod no_providers;
mod openai_compat;
mod sample;
mod service;
mod stream_event;
mod tool_types;

// Custom provider implementations (not OpenAI-compatible).
pub mod anthropic;
pub mod google;

pub use anthropic::AnthropicFactory;
pub use google::GoogleFactory;

pub use backend::{Backend, BackendError};
pub use fake::{FakeLlmServiceFactory, TOOL_LOOP_TRIGGER};
pub use llm_message::LlmMessage;
pub use no_providers::{NO_PROVIDER_ID, NoProvidersAvailableFactory};
pub use openai_compat::{OpenAiCompatibleFactory, OpenAiCompatibleService, ProviderConfig};
pub use sample::SampleLlmServiceFactory;
pub use service::{ChatStream, LlmService, LlmServiceError, LlmServiceFactory, ToolStream};
pub use stream_event::{StopReason, StreamEvent};
pub use tool_types::{ToolCall, ToolDefinition, ToolResult};

/// Rich model metadata returned by provider model listing endpoints.
///
/// Each provider's `list_models` returns a `Vec<ModelInfo>`. The `context_length`
/// is populated when the provider returns it (e.g. OpenRouter's `context_length`,
/// Anthropic's `context_window`, Google's `inputTokenLimit`). It is `None` when
/// the provider does not supply it or the field is missing/null.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    /// The model identifier (e.g. `"openai/gpt-4"`, `"claude-sonnet-4-20250514"`).
    pub id: String,
    /// Maximum context length in tokens, if the provider reports it.
    pub context_length: Option<u32>,
}

#[cfg(test)]
mod fake_tests;
