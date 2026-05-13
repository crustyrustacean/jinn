//! Carries fully loaded session data from the persistence actor back to the bus.
//!
//! Sent by the `SessionPersistenceActor` after loading a session from disk.
//! The component handler receives this command and populates the active session
//! state with the restored data.

use serde::{Deserialize, Serialize};

use crate::feat::session::chat_session::ChatSessionState;
use crate::protocol::CommandMsg;

/// Carries fully loaded session data from the persistence actor back to the bus.
///
/// The component handler uses this to replace the runtime session with the
/// deserialized [`ChatSessionState`].
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct SessionLoadCompleted {
    /// The fully deserialized session from disk.
    pub session: ChatSessionState,
}
