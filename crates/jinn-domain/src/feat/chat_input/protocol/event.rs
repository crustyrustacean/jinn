//! Events produced when a chat entry is added to the conversation.

use serde::{Deserialize, Serialize};

use crate::protocol::ChatEntry;
use jinn_core_types::SessionId;

/// A chat entry was added to the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A chat entry was added to the conversation history.")]
pub struct ChatEntrySubmitted {
    /// The session this entry belongs to.
    pub session_id: SessionId,
    /// The chat entry that was added.
    pub entry: ChatEntry,
}

impl crate::common::bus::BusMessage for ChatEntrySubmitted {}
