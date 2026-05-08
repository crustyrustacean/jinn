//! Persisted session data model.
//!
//! [`PersistedSession`] captures durable session data for JSONL serialization.
//! [`SessionSummary`] is a lightweight subset for startup index scanning.
//!
//! Conversion between [`PersistedSession`] and runtime `ChatSessionState` lives
//! in the binary crate (`session_conversion`) to avoid a circular dependency
//! between this crate and `nullslop-component`.

use std::collections::HashMap;

use jiff::Timestamp;
use nullslop_protocol::{ChatEntry, PromptStrategyId, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Blob key for workflow runtime state.
pub const BLOB_WORKFLOW_STATE: &str = "workflow_state";

/// Blob key for prompt strategy state.
pub const BLOB_STRATEGY_STATE: &str = "strategy_state";

/// A serializable snapshot of a chat session for persistence.
///
/// Contains only durable data — ephemeral runtime state (streaming flags,
/// scroll offset, message queue, etc.) is reconstructed with defaults on load.
///
/// Subsystem state (workflows, strategies) is stored as opaque blobs in a
/// [`HashMap<String, serde_json::Value>`]. Each subsystem owns its blob key
/// and is responsible for (de)serialization. Missing or malformed blobs
/// produce safe defaults — subsystems must handle this defensively.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    /// Unique identifier for this session.
    pub session_id: SessionId,
    /// Human-readable title (derived from first user message).
    pub title: String,
    /// When this session was last modified.
    pub updated_at: Timestamp,
    /// The conversation history.
    pub history: Vec<ChatEntry>,
    /// The active prompt strategy for this session.
    pub active_strategy: PromptStrategyId,
    /// Opaque subsystem state blobs, keyed by well-known constants.
    #[serde(default)]
    pub blobs: HashMap<String, JsonValue>,
}

/// Lightweight session metadata for index scanning.
///
/// Deserializable from a full [`PersistedSession`] JSON line because serde
/// ignores unknown fields by default. Used during startup to build the
/// session index without loading full histories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Unique identifier for this session.
    pub session_id: SessionId,
    /// Human-readable title.
    pub title: String,
    /// When this session was last modified.
    pub updated_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nullslop_protocol::{ChatEntry, PromptStrategyId, SessionId};

    use super::*;

    // --- Test: Serde round-trip ---

    #[rstest::rstest]
    fn persisted_session_round_trips_through_serde() {
        // Given a PersistedSession with history, blobs, workflow state, strategy state.
        let session_id = SessionId::new();
        let mut blobs = HashMap::new();
        blobs.insert(
            BLOB_WORKFLOW_STATE.to_owned(),
            serde_json::json!({"definition": {"version": 1}}),
        );
        blobs.insert(
            BLOB_STRATEGY_STATE.to_owned(),
            serde_json::json!({"compaction_count": 5}),
        );

        let original = PersistedSession {
            session_id: session_id.clone(),
            title: "Test Session".to_owned(),
            updated_at: jiff::Timestamp::now(),
            history: vec![ChatEntry::user("hello"), ChatEntry::assistant("world")],
            active_strategy: PromptStrategyId::sliding_window(),
            blobs,
        };

        // When serialized to JSON and deserialized back.
        let json = serde_json::to_string(&original).expect("serialize");
        let back: PersistedSession = serde_json::from_str(&json).expect("deserialize");

        // Then all fields match.
        assert_eq!(back.session_id, original.session_id);
        assert_eq!(back.title, original.title);
        assert_eq!(back.history.len(), 2);
        assert_eq!(back.active_strategy, original.active_strategy);
        assert!(back.blobs.contains_key(BLOB_WORKFLOW_STATE));
        assert!(back.blobs.contains_key(BLOB_STRATEGY_STATE));
    }

    // --- Test: SessionSummary parses from full snapshot ---

    #[rstest::rstest]
    fn session_summary_parses_from_full_persisted_session_json() {
        // Given a full PersistedSession JSON line with history and blobs.
        let session_id = SessionId::new();
        let full = PersistedSession {
            session_id: session_id.clone(),
            title: "Full Session".to_owned(),
            updated_at: jiff::Timestamp::now(),
            history: vec![ChatEntry::user("hello")],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::from([(
                BLOB_WORKFLOW_STATE.to_owned(),
                serde_json::json!({"key": "value"}),
            )]),
        };
        let json = serde_json::to_string(&full).expect("serialize");

        // When deserializing as SessionSummary.
        let summary: SessionSummary = serde_json::from_str(&json).expect("deserialize summary");

        // Then only session_id, title, updated_at are populated.
        assert_eq!(summary.session_id, session_id);
        assert_eq!(summary.title, "Full Session");
        assert!(summary.updated_at <= jiff::Timestamp::now());
    }
}
