//! Reset a session's chat history.
//!
//! Sent by plugins (via `reset_session` emit verb) to clear a session's
//! conversation history and reset its lifecycle state. Used by judge plugins
//! to start each evaluation with a clean workspace.

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;

/// Reset a session's chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetSessionHistory {
    /// The session whose history should be cleared.
    pub session_id: SessionId,
}

impl crate::common::bus::BusMessage for ResetSessionHistory {}
