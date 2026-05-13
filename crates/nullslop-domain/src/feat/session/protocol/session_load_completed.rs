//! Carries fully loaded session data from the persistence actor back to the bus.
//!
//! Sent by the `SessionPersistenceActor` (in `nullslop-session-actor`) after loading a session from disk.
//! The component handler receives this command and populates the active session
//! state with the restored data.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::feat::session::chat_entry::ChatEntry;
use crate::protocol::CommandMsg;
use crate::protocol::PromptStrategyId;
use crate::protocol::SessionId;

/// Carries fully loaded session data from the persistence actor back to the bus.
///
/// The component handler uses this to populate `ChatSessionState` with restored
/// history, strategy, and subsystem blobs.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct SessionLoadCompleted {
    /// The session that was loaded.
    pub session_id: SessionId,
    /// Human-readable title.
    pub title: String,
    /// The conversation history.
    pub history: Vec<ChatEntry>,
    /// The active prompt strategy for this session.
    pub active_strategy: PromptStrategyId,
    /// The model/provider used in this session.
    pub model: String,
    /// Opaque subsystem state blobs (workflow, strategy, etc.).
    #[serde(default)]
    pub blobs: HashMap<String, JsonValue>,
}
