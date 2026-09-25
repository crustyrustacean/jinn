//! Compatibility export for the lightweight session summary.

pub use jinn_session_store_msg::SessionSummary;

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;
    use crate::feat::session::chat_session::ChatSessionState;
    use jinn_core_types::{ChatEntry, SessionId};
    use jinn_session_store_msg::SessionState;

    #[rstest::rstest]
    fn session_summary_parses_from_full_session_json() {
        // Given a full live session with history and a title.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello"));
        session.set_title("Full Session".to_owned());
        let json = serde_json::to_string(&session).expect("serialize");

        // When deserialized as a lightweight summary.
        let summary: SessionSummary = serde_json::from_str(&json).expect("deserialize");

        // Then identity and title are populated while history is ignored.
        assert_eq!(summary.session_id, *session.session_id());
        assert_eq!(summary.title, "Full Session");
        assert!(summary.parent_session.is_none());
    }

    #[rstest::rstest]
    fn session_summary_deserializes_parent_from_full_session_json() {
        // Given a full live session with a parent.
        let parent_id = SessionId::new();
        let mut session = ChatSessionState::new();
        session.restore_parent_session(Some(parent_id.clone()));
        let json = serde_json::to_string(&session).expect("serialize");

        // When deserialized as a lightweight summary.
        let summary: SessionSummary = serde_json::from_str(&json).expect("deserialize");

        // Then parent metadata is populated.
        assert_eq!(summary.parent_session, Some(parent_id));
    }

    #[rstest::rstest]
    fn session_summary_defaults_session_state_to_loaded() {
        // Given summary JSON without a session_state field.
        let json = r#"{"session_id":"00000000-0000-0000-0000-000000000001","title":"test","updated_at":"2024-01-01T00:00:00Z","created_at":"2024-01-01T00:00:00Z"}"#;

        // When deserialized.
        let summary: SessionSummary = serde_json::from_str(json).expect("deserialize");

        // Then the session defaults to loaded.
        assert_eq!(summary.session_state, SessionState::Loaded);
    }
}
