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
    /// The content width to measure at.
    ///
    /// Carried by the requester rather than re-derived on arrival. The width
    /// belongs to the session that was on screen when the decision was made,
    /// and by the time the actor sees this the frontend has already switched
    /// to the target session — so re-deriving it would measure against the
    /// target's never-rendered width of zero, produce counts no frame can use,
    /// and have the completion actor reject them as stale.
    pub content_width: u16,
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
        // Given a measure request for a session at a width.
        let session_id = SessionId::new();
        let request = ChatLogMeasureRequested {
            session_id: session_id.clone(),
            content_width: 72,
        };

        // When serializing and deserializing it.
        let json = serde_json::to_string(&request).unwrap();
        let round: ChatLogMeasureRequested = serde_json::from_str(&json).unwrap();

        // Then both the session and the width survive.
        assert_eq!(round.session_id, session_id);
        assert_eq!(round.content_width, 72);
    }
}
