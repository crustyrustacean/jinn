//! Remove a session from the sessions map.
//!
//! Sent by the intent handler when closing a session (with or without teardown).
//! The session-persistence actor handles removal, creates a new session if the
//! map is empty, switches the active session if needed, and emits [`SessionRemoved`].
//!
//! [`SessionRemoved`]: super::session_removed::SessionRemoved

use serde::{Deserialize, Serialize};

use crate::protocol::{CommandMsg, SessionId};

/// Remove a session from the sessions map.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct RemoveSession {
    /// The session to remove.
    pub session_id: SessionId,
}
