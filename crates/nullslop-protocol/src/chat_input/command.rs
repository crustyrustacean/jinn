//! Commands that mutate the chat input buffer.
//!
//! Insertion, deletion, submission, and clearing of the text
//! the user is composing.

use serde::{Deserialize, Serialize};

use crate::ChatEntry;
use crate::CommandMsg;
use crate::SessionId;

/// Insert a character into the chat input buffer.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct InsertChar {
    /// The character to insert.
    pub ch: char,
}

/// Delete the last grapheme from the chat input buffer.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct DeleteGrapheme;

/// Submit the chat input buffer as a message.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct SubmitMessage {
    /// The session this message belongs to.
    pub session_id: SessionId,
    /// The message text.
    pub text: String,
}

/// Clear the chat input buffer.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct Clear;

/// Context-sensitive interrupt: clear the input buffer if non-empty, otherwise quit.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct Interrupt;

/// Move the cursor one grapheme to the left.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct MoveCursorLeft;

/// Move the cursor one grapheme to the right.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct MoveCursorRight;

/// Move the cursor to the beginning of the input buffer.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct MoveCursorToStart;

/// Move the cursor to the end of the input buffer.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct MoveCursorToEnd;

/// Delete the grapheme after the cursor (forward delete).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct DeleteGraphemeForward;

/// Move the cursor one word to the left.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct MoveCursorWordLeft;

/// Move the cursor one word to the right.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct MoveCursorWordRight;

/// Move the cursor up one visual line.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct MoveCursorUp;

/// Move the cursor down one visual line.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct MoveCursorDown;

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
    /// The message text to enqueue.
    pub text: String,
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

/// Confirm the autocomplete selection (Tab key in Input scope).
///
/// When autocomplete is active, completes the selected template name.
/// When inactive, falls back to `SwitchTab` so Tab still switches tabs.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct AutocompleteConfirm;

/// Select the next chat entry (toward newer messages).
///
/// If no entry is selected, selects the first.
/// Used by `j` key in Normal mode for entry targeting.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct ChatEntrySelectNext {
    /// The session to navigate.
    pub session_id: SessionId,
}

/// Select the previous chat entry (toward older messages).
///
/// If no entry is selected, selects the last.
/// Used by `k` key in Normal mode for entry targeting.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct ChatEntrySelectPrev {
    /// The session to navigate.
    pub session_id: SessionId,
}

/// Cancel entry selection in the chat log.
///
/// Clears the selection highlight. Used by `Escape` key in Normal mode.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("chat_input")]
pub struct ChatEntrySelectCancel {
    /// The session to clear selection on.
    pub session_id: SessionId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_entry_select_next_serialization_roundtrip() {
        // Given a ChatEntrySelectNext command.
        let cmd = ChatEntrySelectNext {
            session_id: SessionId::new(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: ChatEntrySelectNext = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.session_id, cmd.session_id);
    }

    #[test]
    fn chat_entry_select_next_has_command_name() {
        assert_eq!(ChatEntrySelectNext::NAME, "chat_input::ChatEntrySelectNext");
    }

    #[test]
    fn chat_entry_select_prev_serialization_roundtrip() {
        // Given a ChatEntrySelectPrev command.
        let cmd = ChatEntrySelectPrev {
            session_id: SessionId::new(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: ChatEntrySelectPrev = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.session_id, cmd.session_id);
    }

    #[test]
    fn chat_entry_select_prev_has_command_name() {
        assert_eq!(ChatEntrySelectPrev::NAME, "chat_input::ChatEntrySelectPrev");
    }

    #[test]
    fn chat_entry_select_cancel_serialization_roundtrip() {
        // Given a ChatEntrySelectCancel command.
        let cmd = ChatEntrySelectCancel {
            session_id: SessionId::new(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: ChatEntrySelectCancel = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.session_id, cmd.session_id);
    }

    #[test]
    fn chat_entry_select_cancel_has_command_name() {
        assert_eq!(ChatEntrySelectCancel::NAME, "chat_input::ChatEntrySelectCancel");
    }
}
