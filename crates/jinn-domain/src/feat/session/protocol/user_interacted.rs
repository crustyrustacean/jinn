//! Session was interacted with by the user.
//!
//! Emitted by the session-persistence actor after handling
//! [`MarkSessionInteracted`]. Other actors can subscribe to this event
//! to react to session interaction (e.g., enabling persistence-related features).
//!
//! [`MarkSessionInteracted`]: super::mark_session_interacted::MarkSessionInteracted

use serde::{Deserialize, Serialize};

use crate::protocol::{EventMsg, SessionId};

/// Session was interacted with by the user.
///
/// Broadcast after the session's `has_interacted` flag is set to `true`.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("session")]
pub struct UserInteracted {
    /// The session that was interacted with.
    pub session_id: SessionId,
}
