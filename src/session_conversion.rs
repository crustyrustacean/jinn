//! Conversion between persisted and runtime session state.
//!
//! Free functions that bridge [`PersistedSession`] (serializable storage type)
//! and [`ChatSessionState`] (runtime state). These live in the binary crate
//! because they require visibility into both `nullslop-session` and
//! `nullslop-component` — placing them in either crate would create a circular
//! dependency.

use std::collections::HashMap;

use nsslice_session_management_protocol::{BLOB_STRATEGY_STATE, PersistedSession};
use nsslice_chat_session_protocol::ChatSessionState;
use nullslop_protocol::SessionId;

/// Reconstruct runtime state from a persisted snapshot.
///
/// Ephemeral fields (streaming flags, scroll, queue) get safe defaults.
/// Blobs are deserialized back into subsystem state, with missing or
/// malformed data producing `None` defaults.
///
/// Returns a fully reconstructed [`ChatSessionState`] with strategy
/// state already set — the caller just inserts it into
/// [`AppState`](nullslop_component::app_state::AppState).
#[must_use]
pub fn persisted_into_session(persisted: PersistedSession) -> ChatSessionState {
    let mut session = ChatSessionState::new();

    session.restore_history(persisted.history);
    session.switch_strategy(persisted.active_strategy);

    // Deserialize strategy state blob — missing → None.
    if let Some(strategy_value) = persisted.blobs.get(BLOB_STRATEGY_STATE) {
        session.set_strategy_state(strategy_value.clone());
    }

    session
}

/// Extract durable data from a runtime session, serializing subsystem state
/// into blobs.
///
/// All per-session data (history, strategy state) lives on
/// [`ChatSessionState`], so this is a clean 1:1 extraction. Sets
/// `updated_at` to the current timestamp.
#[must_use]
pub fn session_to_persisted(
    session: &ChatSessionState,
    session_id: &SessionId,
    title: &str,
) -> PersistedSession {
    let mut blobs = HashMap::new();

    if let Some(strategy_state) = session.strategy_state() {
        blobs.insert(BLOB_STRATEGY_STATE.to_owned(), strategy_state.clone());
    }

    use jiff::Timestamp;

    PersistedSession {
        session_id: session_id.clone(),
        title: title.to_owned(),
        updated_at: Timestamp::now(),
        history: session.history().to_vec(),
        active_strategy: session.active_strategy().clone(),
        blobs,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nsslice_session_management_protocol::PersistedSession;
    use nsslice_chat_session_protocol::ChatSessionState;
    use nullslop_protocol::{ChatEntry, PromptStrategyId, SessionId};

    use super::*;

    // --- Test: Missing strategy blob produces None ---

    #[rstest::rstest]
    fn strategy_state_is_none_when_blob_missing() {
        // Given a PersistedSession with empty blobs map.
        let persisted = PersistedSession {
            session_id: SessionId::new(),
            title: "Empty".to_owned(),
            updated_at: jiff::Timestamp::now(),
            history: vec![],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };

        // When calling persisted_into_session.
        let session = persisted_into_session(persisted);

        // Then strategy state is None.
        assert!(session.strategy_state().is_none());
    }

    // --- Test: Session has defaults when blobs are missing ---

    #[rstest::rstest]
    fn session_has_defaults_when_blobs_missing() {
        // Given a PersistedSession with empty blobs map.
        let persisted = PersistedSession {
            session_id: SessionId::new(),
            title: "Empty".to_owned(),
            updated_at: jiff::Timestamp::now(),
            history: vec![],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };

        // When calling persisted_into_session.
        let session = persisted_into_session(persisted);

        // Then session has default values.
        assert!(!session.is_streaming());
        assert!(!session.is_sending());
        assert!(session.queue().is_empty());
    }

    // --- Test: Ephemeral fields reset to defaults on round-trip ---

    #[rstest::rstest]
    fn ephemeral_fields_reset_to_defaults() {
        // Given a ChatSessionState with history entries.
        let mut runtime = ChatSessionState::new();
        runtime.push_entry(ChatEntry::user("hello"));
        runtime.push_entry(ChatEntry::assistant("world"));

        // When converting to PersistedSession and back to ChatSessionState.
        let session_id = SessionId::new();
        let persisted = session_to_persisted(&runtime, &session_id, "Test");
        let restored = persisted_into_session(persisted);

        // Then all ephemeral fields have defaults.
        assert!(!restored.is_streaming());
        assert!(!restored.is_sending());
        assert!(!restored.is_assembling());
        assert!(restored.scroll_offset().is_none());
        assert!(restored.queue().is_empty());
    }

    // --- Test: Durable fields survive round-trip ---

    #[rstest::rstest]
    fn durable_fields_preserved_after_roundtrip() {
        // Given a ChatSessionState with history entries.
        let mut runtime = ChatSessionState::new();
        runtime.push_entry(ChatEntry::user("hello"));
        runtime.push_entry(ChatEntry::assistant("world"));

        // When converting to PersistedSession and back to ChatSessionState.
        let session_id = SessionId::new();
        let persisted = session_to_persisted(&runtime, &session_id, "Test");
        let restored = persisted_into_session(persisted);

        // Then durable fields are preserved.
        assert_eq!(restored.history().len(), 2);
    }

    // --- Test: Strategy state round-trips through blob ---

    #[rstest::rstest]
    fn strategy_state_round_trips_through_persisted_session_blob() {
        // Given a ChatSessionState with strategy state set.
        let mut runtime = ChatSessionState::new();
        runtime.set_strategy_state(serde_json::json!({"compaction_count": 7}));

        // When converting to PersistedSession and back.
        let session_id = SessionId::new();
        let persisted = session_to_persisted(&runtime, &session_id, "Test");
        let restored = persisted_into_session(persisted);

        // Then the strategy state is preserved.
        let blob = restored
            .strategy_state()
            .expect("strategy state should be restored");
        assert_eq!(blob["compaction_count"], 7);
    }
}
