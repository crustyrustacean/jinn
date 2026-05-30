//! Commands that mutate the chat input buffer.
//!
//! Insertion, deletion, submission, and clearing of the text
//! the user is composing.

use serde::{Deserialize, Serialize};

use crate::protocol::ChatEntry;
use crate::protocol::CommandMsg;
use crate::protocol::SessionId;

/// Push a chat entry into the conversation history.
///
/// Any component or actor can send this to add an entry to the chat log.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct PushChatEntry {
    /// The session this entry belongs to.
    pub session_id: SessionId,
    /// The chat entry to add.
    pub entry: ChatEntry,
}

/// Enqueue a user message for processing by the message queue.
///
/// Submitted instead of directly pushing a chat entry when the queue is active.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct EnqueueUserMessage {
    /// The session this message belongs to.
    pub session_id: SessionId,
    /// The fully constructed user chat entry (with display/expanded text).
    pub entry: ChatEntry,
}

/// Set the chat input buffer text directly.
///
/// Used when draining queued messages back into the input box (e.g. on cancel).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct SetChatInputText {
    /// The session whose input buffer to set.
    pub session_id: SessionId,
    /// The new text for the input buffer.
    pub text: String,
}
