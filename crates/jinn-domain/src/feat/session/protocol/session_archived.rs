//! Session archived in persistent storage.
//!
//! Emitted by the session-persistence actor after marking a session as
//! archived in SQLite. This is emitted before [`SessionClosed`] so consumers
//! can distinguish between archived closes and empty-session closes.
//!
//! [`SessionClosed`]: super::session_closed::SessionClosed

use serde::{Deserialize, Serialize};

use crate::protocol::{EventMsg, SessionId};

/// Session archived in persistent storage.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("session")]
pub struct SessionArchived {
    /// The session that was archived.
    pub session_id: SessionId,
}
