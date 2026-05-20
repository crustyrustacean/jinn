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
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::protocol::ChatEntry;

    // --- Test: SessionSummary parses from full ChatSessionState JSON ---

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
    }
}
