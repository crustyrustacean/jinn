//! Events produced when a chat entry is added to the conversation.

use serde::{Deserialize, Serialize};

use crate::protocol::ChatEntry;
use crate::protocol::EventMsg;
use crate::protocol::SessionId;

/// A chat entry was added to the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("chat_input")]
pub struct ChatEntrySubmitted {
    /// The session this entry belongs to.
    pub session_id: SessionId,
    /// The chat entry that was added.
    pub entry: ChatEntry,
}

impl crate::common::bus::BusMessage for ChatEntrySubmitted {}
