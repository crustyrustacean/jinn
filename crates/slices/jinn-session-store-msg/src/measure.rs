//! Commands asking the session store to measure a session's chat log.

use jinn_core_types::SessionId;
use serde::{Deserialize, Serialize};

/// Measure a session's chat log off the render thread.
///
/// Sent when switching to a session that is already in memory but whose line
/// counts are not cached. Such a session needs no disk read — only the same
/// measurement a session restored from the archive requires, and without it the
/// next frame lays the whole history out inline.
///
/// Deliberately separate from [`SessionLoadRequested`]: routing through a load
/// would re-read a session that is already in the map, which is exactly the
/// work this command exists to avoid.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Measure the chat log line counts for an in-memory session.")]
pub struct ChatLogMeasureRequested {
    /// The session whose chat log should be measured.
    pub session_id: SessionId,
}

impl jinn_slices::BusMessage for ChatLogMeasureRequested {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::ChatLogMeasureRequested;
    use jinn_core_types::SessionId;

    #[rstest::rstest]
    #[test]
    fn measure_requested_roundtrips_through_json() {
        // Given a measure request for a session.
        let session_id = SessionId::new();
        let request = ChatLogMeasureRequested {
            session_id: session_id.clone(),
        };

        // When serializing and deserializing it.
        let json = serde_json::to_string(&request).unwrap();
        let round: ChatLogMeasureRequested = serde_json::from_str(&json).unwrap();

        // Then the session id survives.
        assert_eq!(round.session_id, session_id);
    }
}
