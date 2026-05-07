//! Request to load a full session from disk by byte offset.
//!
//! Emitted when the user confirms a session selection in the session picker.
//! The persistence actor receives this event, seeks to the byte offset,
//! and sends back a [`SessionLoadCompleted`] command with the full session data.
//!
//! [`SessionLoadCompleted`]: crate::session::SessionLoadCompleted

use serde::{Deserialize, Serialize};

use crate::EventMsg;
use crate::session::SessionId;

/// Request to load a full session from disk by byte offset.
///
/// Carries the session ID and byte offset so the actor can seek directly
/// to the right line in the JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("session")]
pub struct SessionLoadRequested {
    /// The session to load.
    pub session_id: SessionId,
    /// Byte offset in the JSONL file where the session line starts.
    pub byte_offset: u64,
}
