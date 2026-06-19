//! Lightweight session metadata for index scanning.
//!
//! [`SessionSummary`] is a small subset of session fields used during startup
//! to build the session index without loading full histories. Deserializable
//! from a full [`ChatSessionState`](super::chat_session::ChatSessionState) JSON
//! line because serde ignores unknown fields by default.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::feat::session::chat_session::SessionState;
use crate::protocol::SessionId;

/// Lightweight session metadata for index scanning.
///
/// Deserializable from a full [`ChatSessionState`](super::chat_session::ChatSessionState)
/// JSON line because serde ignores unknown fields by default. Used during startup
/// to build the session index without loading full histories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Unique identifier for this session.
    pub session_id: SessionId,
    /// Human-readable title. Defaults to "Untitled Session" for sessions
    /// without a title (new sessions or sessions from before title tracking).
    #[serde(default = "default_title")]
    pub title: String,
    /// When this session was last modified.
    pub updated_at: Timestamp,
    /// When this session was created. Set once at construction, never mutated.
    #[serde(default = "default_timestamp")]
    pub created_at: Timestamp,
    /// Whether this session is loaded in memory or archived.
    #[serde(default = "default_session_state")]
    pub session_state: SessionState,
    /// Parent session ID - `None` for root sessions.
    ///
    /// Deserialized from stored `ChatSessionState` JSON; defaults to `None`
    /// for sessions created before parent tracking was added.
    #[serde(default)]
    pub parent_session: Option<SessionId>,
}

fn default_title() -> String {
    "Untitled Session".to_owned()
}

fn default_timestamp() -> Timestamp {
    Timestamp::now()
}

fn default_session_state() -> SessionState {
    SessionState::Loaded
}

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
    use crate::protocol::ChatEntry;

    #[rstest::rstest]
    fn session_summary_parses_from_full_chat_session_state_json() {
        // Given a full ChatSessionState JSON line with history.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello"));
        session.set_title("Full Session".to_owned());
        let json = serde_json::to_string(&session).expect("serialize");

        // When deserializing as SessionSummary.
        let summary: SessionSummary = serde_json::from_str(&json).expect("deserialize summary");

        // Then session_id, title, updated_at are populated.
        assert_eq!(summary.session_id, *session.session_id());
        assert_eq!(summary.title, "Full Session");
        assert!(summary.updated_at <= jiff::Timestamp::now());
        // And parent_session defaults to None (session has no parent).
        assert!(summary.parent_session.is_none());
    }

    #[rstest::rstest]
    fn session_summary_deserializes_parent_session_from_json() {
        // Given a ChatSessionState with a parent session set.
        let parent_id = SessionId::new();
        let mut session = ChatSessionState::new();
        session.set_title("Child Session".to_owned());
        session.restore_parent_session(Some(parent_id.clone()));
        let json = serde_json::to_string(&session).expect("serialize");

        // When deserializing as SessionSummary.
        let summary: SessionSummary = serde_json::from_str(&json).expect("deserialize summary");

        // Then parent_session is populated.
        assert_eq!(summary.parent_session, Some(parent_id));
    }

    #[rstest::rstest]
    fn session_summary_defaults_parent_session_to_none() {
        // Given JSON without a parent_session field.
        let json = r#"{"session_id":"abc","title":"test","updated_at":"2024-01-01T00:00:00Z","created_at":"2024-01-01T00:00:00Z","session_state":"loaded"}"#;

        // When deserializing.
        let summary: SessionSummary = serde_json::from_str(json).expect("deserialize");

        // Then parent_session defaults to None.
        assert!(summary.parent_session.is_none());
    }

    #[rstest::rstest]
    fn session_summary_defaults_title_to_untitled() {
        // Given JSON without a title field.
        let json = r#"{"session_id":"abc","updated_at":"2024-01-01T00:00:00Z","created_at":"2024-01-01T00:00:00Z","session_state":"loaded"}"#;

        // When deserializing.
        let summary: SessionSummary = serde_json::from_str(json).expect("deserialize");

        // Then title defaults to "Untitled Session".
        assert_eq!(summary.title, "Untitled Session");
    }

    #[rstest::rstest]
    fn session_summary_defaults_session_state_to_loaded() {
        // Given JSON without a session_state field.
        let json = r#"{"session_id":"abc","title":"test","updated_at":"2024-01-01T00:00:00Z","created_at":"2024-01-01T00:00:00Z"}"#;

        // When deserializing.
        let summary: SessionSummary = serde_json::from_str(json).expect("deserialize");

        // Then session_state defaults to Loaded.
        assert_eq!(summary.session_state, super::SessionState::Loaded);
    }
}
