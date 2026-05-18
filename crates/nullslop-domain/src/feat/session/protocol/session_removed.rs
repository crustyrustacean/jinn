//! Session removed from the sessions map.
//!
//! Emitted by the session-persistence actor after removing a session from the
//! map (via [`RemoveSession`] command or teardown success). Sidebar actors
//! subscribe to this event to adjust cursor state.

use serde::{Deserialize, Serialize};

use crate::protocol::{EventMsg, SessionId};

/// Session removed from the sessions map.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("session")]
pub struct SessionRemoved {
    /// The session that was removed.
    pub session_id: SessionId,
}
