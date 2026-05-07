//! Carries fully loaded session data from the persistence actor back to the bus.
//!
//! Sent by the [`SessionPersistenceActor`] after loading a session from disk.
//! The component handler receives this command and populates the active session
//! state with the restored data.
//!
//! [`SessionPersistenceActor`]: nullslop_session_actor::SessionPersistenceActor

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::CommandMsg;
use crate::chat::ChatEntry;
use crate::context::PromptStrategyId;
use crate::session::SessionId;

/// Carries fully loaded session data from the persistence actor back to the bus.
///
/// The component handler uses this to populate [`ChatSessionState`] with restored
/// history, strategy, and subsystem blobs.
///
/// [`ChatSessionState`]: nullslop_component::ChatSessionState
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
    /// Opaque subsystem state blobs (workflow, strategy, etc.).
    #[serde(default)]
    pub blobs: HashMap<String, JsonValue>,
}
