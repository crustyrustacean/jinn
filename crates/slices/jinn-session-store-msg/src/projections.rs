//! Lightweight session metadata and archived tree projections.

use std::path::PathBuf;

use jiff::Timestamp;
use jinn_core_types::SessionId;
use serde::{Deserialize, Serialize};

use crate::SessionState;

/// Lightweight metadata used to build a session index without full history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Unique identifier for this session.
    pub session_id: SessionId,
    /// Human-readable title.
    #[serde(default = "default_title")]
    pub title: String,
    /// When this session was last modified.
    pub updated_at: Timestamp,
    /// When this session was created.
    #[serde(default = "default_timestamp")]
    pub created_at: Timestamp,
    /// Whether this session is loaded in memory or archived.
    #[serde(default = "default_session_state")]
    pub session_state: SessionState,
    /// Parent session ID, or `None` for a root session.
    #[serde(default)]
    pub parent_session: Option<SessionId>,
    /// Project directory associated with this session.
    #[serde(default)]
    pub project: Option<PathBuf>,
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

/// A lightweight snapshot of an archived session's aggregate statistics.
#[derive(Debug, Clone)]
pub struct FrozenTreeNode {
    /// The archived session's ID.
    pub session_id: SessionId,
    /// Parent session ID, or `None` for a root session.
    pub parent_session_id: Option<SessionId>,
    /// Total tokens sent across requests in this session.
    pub total_sent: u64,
    /// Total tokens received across responses in this session.
    pub total_received: u64,
    /// Total cost in USD across requests in this session.
    pub total_cost: f64,
    /// Total user-message turns in this session.
    pub total_turns: u32,
    /// Effective sent total.
    pub effective_sent: u64,
    /// Sum of provider-reported prompt-token counts.
    pub measured_sent: u64,
    /// Sum of provider-reported cache-hit counts.
    pub cached_total: u64,
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

    #[rstest::rstest]
    fn summary_defaults_missing_legacy_fields() {
        // Given summary JSON without optional legacy fields.
        let json = r#"{"session_id":"00000000-0000-0000-0000-000000000001","updated_at":"2024-01-01T00:00:00Z"}"#;

        // When deserialized.
        let summary: SessionSummary = serde_json::from_str(json).expect("deserialize");

        // Then all optional fields receive their compatibility defaults.
        assert_eq!(summary.title, "Untitled Session");
        assert_eq!(summary.session_state, SessionState::Loaded);
        assert!(summary.parent_session.is_none());
        assert!(summary.project.is_none());
    }

    #[rstest::rstest]
    fn summary_roundtrips_project_metadata() {
        // Given a summary with project and parent metadata.
        let session_id = SessionId::new();
        let parent_id = SessionId::new();
        let summary = SessionSummary {
            session_id: session_id.clone(),
            title: "Child".to_owned(),
            updated_at: Timestamp::UNIX_EPOCH,
            created_at: Timestamp::UNIX_EPOCH,
            session_state: SessionState::Loaded,
            parent_session: Some(parent_id.clone()),
            project: Some(PathBuf::from("/repo")),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&summary).expect("serialize");
        let restored: SessionSummary = serde_json::from_str(&json).expect("deserialize");

        // Then the complete projection survives.
        assert_eq!(restored.session_id, session_id);
        assert_eq!(restored.parent_session, Some(parent_id));
        assert_eq!(restored.project, Some(PathBuf::from("/repo")));
        assert_eq!(restored.title, "Child");
    }
}
