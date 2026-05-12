//! Provider commands.

use serde::{Deserialize, Serialize};

use crate::feat::provider::llm_message::LlmMessage;
use crate::protocol::CommandMsg;
use crate::protocol::SessionId;

/// Switch the active LLM provider.
///
/// Carries the target provider ID. The handler validates it against the registry,
/// swaps the factory, and emits [`ProviderSwitched`](super::ProviderSwitched).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct ProviderSwitch {
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
    /// Optional provider override for per-message routing (future).
    /// Currently always `None` — uses the active provider.
    #[serde(default)]
    pub provider_id: Option<String>,
}

/// Refresh the model list from all providers.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct RefreshModels;

/// Rescan the prompt templates directory and reload templates.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct RescanPromptTemplates;

/// Load entries for the provider/model picker.
///
/// The provider actor receives this, loads entries from the provider registry,
/// and writes them into `AppState`.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("provider")]
pub struct LoadProviderPickerEntries;

#[cfg(test)]
mod tests {
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
