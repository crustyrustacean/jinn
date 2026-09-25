//! Commands that mutate the chat input buffer.
//!
//! Insertion, deletion, submission, and clearing of the text
//! the user is composing.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use jinn_core_types::{ChatEntry, SessionId};
use jinn_slices::BusMessage;

/// Enqueue a user message for processing by the message queue.
///
/// Submitted instead of directly pushing a chat entry when the queue is active.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Enqueue a fully built user entry for dispatch.")]
pub struct EnqueueUserMessage {
    /// The session this message belongs to.
    pub session_id: SessionId,
    /// The fully constructed user chat entry (with display/expanded text).
    pub entry: ChatEntry,
}

impl BusMessage for EnqueueUserMessage {}

/// Enqueue a manual resume for a session: re-assemble current history and
/// re-send to the provider. Adds no user message.
///
/// Submitted instead of pushing a fresh user entry when the user wants to
/// resume after an error or after restarting the app mid-stream.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Resume dispatching turns for an idle session.")]
pub struct EnqueueResumeTurn {
    /// The session to resume.
    pub session_id: SessionId,
}

impl BusMessage for EnqueueResumeTurn {}

/// Append a fragment to a session's steering buffer.
///
/// Submitted when the user picks STEER mode and the LLM is currently
/// mid-turn (phase != Idle). Fragments accumulate FIFO and are drained
/// into a single `User` entry at the next prompt-assembly boundary.
///
/// If submitted while phase == Idle, the chat-input layer is responsible
/// for routing to [`EnqueueUserMessage`] instead.
///
/// See `jinn_session_state::steering_buffer::SteeringBuffer`.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Append a steering fragment to a busy session.")]
pub struct SubmitSteeringMessage {
    /// The session whose steering buffer to append to.
    pub session_id: SessionId,
    /// The raw user-typed text to buffer.
    pub text: String,
}

impl BusMessage for SubmitSteeringMessage {}

/// Command: list the directory at `path` (already resolved absolute) for the
/// active session's `@path` popup.
///
/// `request_id` is the staleness token. The actor writes its result only when
/// this matches `frontend.file_picker.expected_request_id`, so an earlier,
/// slow read cannot overwrite a newer one.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "List a directory for the file picker popup.")]
pub struct ListDirectory {
    /// The session whose popup this listing is for.
    pub session_id: SessionId,
    /// Resolved absolute directory to list.
    pub path: PathBuf,
    /// Monotonic id tying this request to the expected reply slot.
    pub request_id: u64,
}

impl BusMessage for ListDirectory {}
