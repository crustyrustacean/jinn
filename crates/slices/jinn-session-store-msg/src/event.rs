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

/// Emitted after a session's library membership changes.
///
/// Library membership is a durable flag on the session, so this event tells
/// consumers (the sidebar's Sessions and Library sections) that their view of
/// which sessions are members — and therefore which appear in which list — is
/// stale.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session's library membership changed.")]
pub struct SessionLibraryMembershipChanged {
    /// The session whose membership changed.
    pub session_id: SessionId,
    /// The session's new membership state.
    pub in_library: bool,
}

impl jinn_slices::BusMessage for SessionLibraryMembershipChanged {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::{SessionLibraryMembershipChanged, SessionLoadCompleted};
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

    #[rstest::rstest]
    #[test]
    fn library_membership_changed_roundtrips() {
        // Given a membership-change event.
        let event = SessionLibraryMembershipChanged {
            session_id: SessionId::new(),
            in_library: true,
        };

        // When serializing and deserializing the event.
        let json = serde_json::to_string(&event).unwrap();
        let restored: SessionLibraryMembershipChanged = serde_json::from_str(&json).unwrap();

        // Then both fields survive unchanged.
        assert_eq!(restored.session_id, event.session_id);
        assert!(restored.in_library);
    }
}
