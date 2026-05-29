//! Carries fully loaded session data from the persistence actor back to the bus.
//!
//! Emitted by the `SessionPersistenceActor` after loading a session from disk.
//! Multiple actors can subscribe to this event (session-actor, token-count actor, etc.)
//! to perform post-load initialization.

use serde::{Deserialize, Serialize};

use crate::feat::session::chat_session::ChatSessionState;
use crate::protocol::EventMsg;

/// Emitted when a session has been fully loaded from persistent storage.
///
/// The session-actor uses this to replace the runtime session with the
/// deserialized [`ChatSessionState`]. The token-count actor uses this to
/// trigger batch token count computation for all entries in the loaded session.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("session")]
pub struct SessionLoadCompleted {
    /// The fully deserialized session from disk.
    pub session: ChatSessionState,
}
