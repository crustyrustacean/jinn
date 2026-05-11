//! Chat input domain: commands and events for the text input box.
//!
//! Users type into the input box to compose messages; these types
//! model the resulting edits and submissions.

pub use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
pub use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
