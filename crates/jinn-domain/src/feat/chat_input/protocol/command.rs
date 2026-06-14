//! Commands that mutate the chat input buffer.
//!
//! Insertion, deletion, submission, and clearing of the text
//! the user is composing.

use error_stack::ResultExt;
use serde::{Deserialize, Serialize};

use crate::BusMessage;
use crate::protocol::ChatEntry;
use crate::protocol::SessionId;

/// Push a chat entry into the conversation history.
///
/// Any component or actor can send this to add an entry to the chat log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushChatEntry {
    /// The session this entry belongs to.
    pub session_id: SessionId,
    /// The chat entry to add.
    pub entry: ChatEntry,
}

impl crate::common::bus::BusMessage for PushChatEntry {}

impl crate::common::plugin_bridge::TryFromLua for PushChatEntry {
    const VERB: &'static str = "push_chat_entry";

    fn try_from_lua(
        ctx: crate::common::plugin_bridge::CmdCtx,
        data: serde_json::Value,
    ) -> Result<Self, error_stack::Report<crate::common::plugin_bridge::PluginBridgeError>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum LuaChatEntryKind {
            System(String),
            Transient(String),
            Error(String),
        }

        #[derive(Deserialize)]
        struct LuaPayload {
            session_id: SessionId,
            kind: LuaChatEntryKind,
        }

        let lua: LuaPayload = serde_json::from_value(data)
            .change_context(crate::common::plugin_bridge::PluginBridgeError)
            .attach(ctx)
            .attach("deserialize push_chat_entry payload")?;

        let entry = match lua.kind {
            LuaChatEntryKind::System(text) => ChatEntry::system(text),
            LuaChatEntryKind::Transient(text) => ChatEntry::transient(text),
            LuaChatEntryKind::Error(text) => ChatEntry::error(text),
        };

        Ok(PushChatEntry {
            session_id: lua.session_id,
            entry,
        })
    }
}

/// Enqueue a user message for processing by the message queue.
///
/// Submitted instead of directly pushing a chat entry when the queue is active.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnqueueUserMessage {
    /// The session this message belongs to.
    pub session_id: SessionId,
    /// The fully constructed user chat entry (with display/expanded text).
    pub entry: ChatEntry,
}

impl BusMessage for EnqueueUserMessage {}

impl crate::common::plugin_bridge::TryFromLua for EnqueueUserMessage {
    const VERB: &'static str = "enqueue_user_message";

    fn try_from_lua(
        ctx: crate::common::plugin_bridge::CmdCtx,
        data: serde_json::Value,
    ) -> Result<Self, error_stack::Report<crate::common::plugin_bridge::PluginBridgeError>> {
        #[derive(Deserialize)]
        struct LuaPayload {
            session_id: SessionId,
            text: String,
        }

        let lua: LuaPayload = serde_json::from_value(data)
            .change_context(crate::common::plugin_bridge::PluginBridgeError)
            .attach(ctx)
            .attach("deserialize enqueue_user_message payload")?;

        Ok(EnqueueUserMessage {
            session_id: lua.session_id,
            entry: ChatEntry::user(lua.text),
        })
    }
}

/// Enqueue a manual resume for a session: re-assemble current history and
/// re-send to the provider. Adds no user message.
///
/// Submitted instead of pushing a fresh user entry when the user wants to
/// resume after an error or after restarting the app mid-stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnqueueResumeTurn {
    /// The session to resume.
    pub session_id: SessionId,
}

impl BusMessage for EnqueueResumeTurn {}
impl BusMessage for SetChatInputText {}
/// Append a fragment to a session's steering buffer.
///
/// Submitted when the user picks STEER mode and the LLM is currently
/// mid-turn (phase != Idle). Fragments accumulate FIFO and are drained
/// into a single `User` entry at the next prompt-assembly boundary.
///
/// If submitted while phase == Idle, the chat-input layer is responsible
/// for routing to [`EnqueueUserMessage`] instead.
///
/// See [`crate::feat::session::steering_buffer::SteeringBuffer`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitSteeringMessage {
    /// The session whose steering buffer to append to.
    pub session_id: SessionId,
    /// The raw user-typed text to buffer.
    pub text: String,
}

impl crate::common::bus::BusMessage for SubmitSteeringMessage {}

/// Set the chat input buffer text directly.
///
/// Used when draining queued messages back into the input box (e.g. on cancel).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetChatInputText {
    /// The session whose input buffer to set.
    pub session_id: SessionId,
    /// The new text for the input buffer.
    pub text: String,
}

impl crate::common::plugin_bridge::TryFromLua for SetChatInputText {
    const VERB: &'static str = "set_chat_input";

    fn try_from_lua(
        ctx: crate::common::plugin_bridge::CmdCtx,
        data: serde_json::Value,
    ) -> Result<Self, error_stack::Report<crate::common::plugin_bridge::PluginBridgeError>> {
        #[derive(Deserialize)]
        struct LuaPayload {
            session_id: SessionId,
            text: String,
        }

        let lua: LuaPayload = serde_json::from_value(data)
            .change_context(crate::common::plugin_bridge::PluginBridgeError)
            .attach(ctx)
            .attach("deserialize set_chat_input payload")?;

        Ok(SetChatInputText {
            session_id: lua.session_id,
            text: lua.text,
        })
    }
}
