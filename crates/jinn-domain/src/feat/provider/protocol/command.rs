//! Provider commands.

use serde::{Deserialize, Serialize};

use crate::feat::provider::llm_message::LlmMessage;
use crate::feat::tools_actor::tool_types::ToolDefinition;
use crate::protocol::{CommandMsg, SessionId};

/// Switch the active LLM provider.
///
/// Carries the target provider ID. The handler validates it against the registry,
/// swaps the factory, and emits [`ProviderSwitched`](super::ProviderSwitched).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct ProviderSwitch {
    /// The session whose model should be switched.
    pub session_id: SessionId,
    /// The provider to switch to.
    pub provider_id: String,
}

/// Send a message to the AI provider.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct SendMessage {
    /// The session this message belongs to.
    pub session_id: SessionId,
    /// The message text.
    pub text: String,
}

/// Cancel the active provider stream for a session.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct CancelStream {
    /// The session whose stream should be cancelled.
    pub session_id: SessionId,
}

/// Command to send conversation context to the LLM provider.
///
/// Emitted by `LlmRequestHandler` when a user message is submitted.
/// Carries the full conversation history as pre-converted messages.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct SendToLlmProvider {
    /// The session this request belongs to.
    pub session_id: SessionId,
    /// The full conversation history, converted to LLM messages.
    pub messages: Vec<LlmMessage>,
    /// Tool definitions available for the LLM to call.
    #[serde(default)]
    pub tool_definitions: Vec<ToolDefinition>,
    /// Optional provider override for per-message routing (future).
    /// Currently always `None` - uses the active provider.
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Estimated token count of all messages + tool schemas.
    #[serde(default)]
    pub estimated_tokens: u32,
}

impl crate::common::bus::BusMessage for SendToLlmProvider {}

/// Refresh the model list from all providers.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct RefreshModels;

/// Rescan prompt templates for a specific session.
///
/// The actor reads the session's cwd, scans user/system plus project-local
/// `.agents/prompts` dirs (most-local wins), and emits `PromptTemplatesLoaded`.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct RescanPromptTemplates {
    /// The session whose cwd drives the scan.
    pub session_id: crate::SessionId,
}

/// Load entries for the provider/model picker.
///
/// The provider actor receives this, loads entries from the provider registry,
/// and writes them into `AppState`.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct LoadProviderPickerEntries;

/// The provider actor receives this, loads compaction model entries (provider
/// entries + a "session default" sentinel) and writes them into `AppState`.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct LoadCompactionModelPickerEntries;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn send_to_llm_provider_deserializes_without_provider_id() {
        // Given JSON without the provider_id field (old format).
        let json = r#"{"session_id":"sid-1","messages":[]}"#;

        // When deserializing.
        let cmd: SendToLlmProvider = serde_json::from_str(json).expect("deserialize");

        // Then provider_id is None (backwards compatible).
        assert!(cmd.provider_id.is_none());
    }
}
