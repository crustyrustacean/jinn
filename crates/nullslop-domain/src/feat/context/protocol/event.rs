//! Event types for prompt assembly.

use serde::{Deserialize, Serialize};

use crate::feat::provider::llm_message::LlmMessage;
use crate::protocol::EventMsg;
use crate::protocol::SessionId;

/// Emitted when a prompt has been assembled and is ready to send.
///
/// The message queue handler receives this event, finishes assembling,
/// and submits the messages to the LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("context")]
pub struct PromptAssembled {
    /// The session this assembly is for.
    pub session_id: SessionId,
    /// System prompt, if any. Should be prepended as `LlmMessage::System`.
    pub system_prompt: Option<String>,
    /// The assembled messages ready for the LLM.
    pub messages: Vec<LlmMessage>,
}

/// Emitted when a chat entry has been pinned or unpinned.
///
/// The context actor emits this after mutating pin state in `AppState`.
/// The session actor subscribes to this event and persists the updated
/// session to disk.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("context")]
pub struct ChatEntryPinChanged {
    /// The session whose pin state changed.
    pub session_id: SessionId,
}

/// Emitted when personas have been scanned and loaded from disk.
///
/// The context actor receives this event and stores the loaded personas
/// in `AppState`. If no active persona is set, the first one becomes default.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("context")]
pub struct PersonasLoaded {
    /// The loaded persona files.
    pub personas: Vec<crate::feat::persona::Persona>,
    /// Error message if scanning failed, `None` on success.
    pub error: Option<String>,
}
