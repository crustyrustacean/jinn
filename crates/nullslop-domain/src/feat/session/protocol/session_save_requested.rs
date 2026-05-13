//! Request to persist the current session state to disk.
//!
//! Emitted by the message queue handler after user message submission and
//! stream completion. The actor receives this event, constructs a
//! `PersistedSession`, and writes it to the session store.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::feat::session::chat_entry::ChatEntry;
use crate::protocol::EventMsg;
use crate::protocol::PromptStrategyId;
use crate::protocol::SessionId;

/// Request to persist the current session state to disk.
///
/// Carries all data needed for persistence — the actor does not access
/// `AppState`. The `updated_at` field is set by the actor at save time.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("session")]
pub struct SessionSaveRequested {
    /// The session to persist.
    pub session_id: SessionId,
    /// Human-readable title (derived from first user message).
    pub title: String,
    /// The conversation history at save time.
    pub history: Vec<ChatEntry>,
    /// The active prompt strategy for this session.
    pub active_strategy: PromptStrategyId,
    /// The model/provider used in this session (e.g., "ollama/llama3").
    pub model: String,
    /// Opaque subsystem state blobs (workflow, strategy, etc.).
    #[serde(default)]
    pub blobs: HashMap<String, JsonValue>,
}
