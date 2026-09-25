//! Session-store events published after live-state transitions complete.

use jinn_core_types::SessionId;
use serde::{Deserialize, Serialize};

/// Emitted after a session is fully initialized and inserted into live state.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session was loaded, initialized, and inserted into state.")]
pub struct SessionLoadCompleted {
    /// The fully initialized session's ID.
    pub session_id: SessionId,
}

impl SessionLoadCompleted {
    /// Returns the completed session ID.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
}

impl jinn_slices::BusMessage for SessionLoadCompleted {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::SessionLoadCompleted;
    use jinn_core_types::SessionId;

    #[rstest::rstest]
    #[test]
    fn load_completed_roundtrips_session_id() {
        // Given a fully initialized session id.
        let event = SessionLoadCompleted {
            session_id: SessionId::new(),
        };

        // When serializing and deserializing the event.
        let json = serde_json::to_string(&event).unwrap();
        let restored: SessionLoadCompleted = serde_json::from_str(&json).unwrap();

        // Then the id survives unchanged.
        assert_eq!(restored.session_id, event.session_id);
    }
}
