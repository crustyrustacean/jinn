//! Event types for prompt assembly.

use serde::{Deserialize, Serialize};

use crate::feat::context::protocol::strategy_id::PromptStrategyId;
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

/// Emitted when a session's prompt assembly strategy has been switched.
///
/// Emitted by the `PromptAssemblyActor` after successfully switching
/// a session to a new strategy.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("context")]
pub struct PromptStrategySwitched {
    /// The session whose strategy was switched.
    pub session_id: SessionId,
    /// The new strategy that is now active.
    pub strategy_id: PromptStrategyId,
}

/// Emitted when a strategy's session state has changed and should be persisted.
///
/// The component handler stores the blob in `AppState` for later restoration.
/// The host doesn't interpret the blob — it just stores and restores it.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("context")]
pub struct StrategyStateUpdated {
    /// The session whose strategy state changed.
    pub session_id: SessionId,
    /// The strategy the state belongs to.
    pub strategy_id: PromptStrategyId,
    /// The opaque state blob to persist.
    pub blob: serde_json::Value,
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
