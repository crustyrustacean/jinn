//! Conversion between persisted and runtime session state.
//!
//! Free functions that bridge [`PersistedSession`] (serializable storage type)
//! and [`ChatSessionState`] (runtime state). These live in the binary crate
//! because they require visibility into both `nullslop-session` and
//! `nullslop-component` — placing them in either crate would create a circular
//! dependency.

use std::collections::HashMap;

use nullslop_component::chat_session::ChatSessionState;
use nullslop_protocol::SessionId;
use nullslop_session::{BLOB_STRATEGY_STATE, BLOB_WORKFLOW_STATE, PersistedSession};
use nullslop_workflow::WorkflowState;

/// Reconstruct runtime state from a persisted snapshot.
///
/// Ephemeral fields (streaming flags, scroll, queue) get safe defaults.
/// Blobs are deserialized back into subsystem state, with missing or
/// malformed data producing `None` defaults.
///
/// Returns a fully reconstructed [`ChatSessionState`] with workflow and
/// strategy state already set — the caller just inserts it into
/// [`AppState`](nullslop_component::app_state::AppState).
#[must_use]
pub fn persisted_into_session(persisted: PersistedSession) -> ChatSessionState {
    let mut session = ChatSessionState::new();

    session.restore_history(persisted.history);
    session.switch_strategy(persisted.active_strategy);

    // Deserialize workflow state blob — missing or malformed → None.
    if let Some(workflow_value) = persisted
        .blobs
        .get(BLOB_WORKFLOW_STATE)
        .and_then(|v| serde_json::from_value::<WorkflowState>(v.clone()).ok())
    {
        session.set_workflow(workflow_value);
    }

    // Deserialize strategy state blob — missing → None.
    if let Some(strategy_value) = persisted.blobs.get(BLOB_STRATEGY_STATE) {
        session.set_strategy_state(strategy_value.clone());
    }

    session
}

/// Extract durable data from a runtime session, serializing subsystem state
/// into blobs.
///
/// All per-session data (history, workflow state, strategy state) lives on
/// [`ChatSessionState`], so this is a clean 1:1 extraction. Sets
/// `updated_at` to the current timestamp.
#[must_use]
pub fn session_to_persisted(
    session: &ChatSessionState,
    session_id: &SessionId,
    title: &str,
) -> PersistedSession {
    let mut blobs = HashMap::new();

    if let Some(workflow) = session.workflow() {
        // WorkflowState derives Serialize, so to_value should never fail.
        // If it does, the workflow blob is silently skipped.
        if let Ok(value) = serde_json::to_value(workflow) {
            blobs.insert(BLOB_WORKFLOW_STATE.to_owned(), value);
        }
    }

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

    use nullslop_component::chat_session::ChatSessionState;
    use nullslop_protocol::{ChatEntry, PromptStrategyId, SessionId};
    use nullslop_session::PersistedSession;
    use nullslop_workflow::{GuardExpr, ModelHint, StepDef, WorkflowDef, WorkflowState};

    use super::*;

    /// Creates a minimal workflow definition for testing.
    fn make_workflow(step_count: usize) -> WorkflowDef {
        let steps: Vec<StepDef> = (0..step_count)
            .map(|i| StepDef {
                id: format!("step-{i}"),
                title: format!("Step {i}"),
                instructions: format!("Instructions for step {i}"),
                model_hint: ModelHint::Small,
                checkpoint: false,
                requires_user_input: false,
                tools: vec![],
                guards: GuardExpr::None,
                outputs: vec![],
                depends_on: vec![],
            })
            .collect();

        WorkflowDef {
            version: 1,
            name: "test-workflow".to_owned(),
            description: "A test workflow".to_owned(),
            model_overrides: HashMap::new(),
            globals: HashMap::new(),
            steps,
        }
    }

    // --- Test: Missing workflow blob produces None ---

    #[rstest::rstest]
    fn workflow_state_is_none_when_blob_missing() {
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

        // Then workflow state is None.
        assert!(session.workflow().is_none());
    }

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

    // --- Test: Workflow state round-trips through blob ---

    #[rstest::rstest]
    fn workflow_state_round_trips_through_persisted_session_blob() {
        // Given a ChatSessionState with a workflow set.
        let mut runtime = ChatSessionState::new();
        let def = make_workflow(2);
        let mut ws = WorkflowState::new(def);
        ws.start().unwrap();
        runtime.set_workflow(ws);

        // When converting to PersistedSession and back.
        let session_id = SessionId::new();
        let persisted = session_to_persisted(&runtime, &session_id, "Test");
        let restored = persisted_into_session(persisted);

        // Then the workflow state is fully reconstructed.
        let workflow = restored.workflow().expect("workflow should be restored");
        assert_eq!(workflow.active_step.as_deref(), Some("step-0"));
        assert_eq!(workflow.steps.len(), 2);
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

    // --- Test: Malformed workflow blob produces None ---

    #[rstest::rstest]
    fn malformed_workflow_blob_produces_none() {
        // Given a PersistedSession JSON with a malformed workflow_state blob.
        let json = r#"{
            "session_id": "s-00000000-0000-0000-0000-000000000001",
            "title": "Bad Blob",
            "updated_at": "2025-01-01T00:00:00Z",
            "history": [],
            "active_strategy": "passthrough",
            "blobs": {
                "workflow_state": "not a valid workflow object"
            }
        }"#;

        // When deserializing and calling persisted_into_session.
        let persisted: PersistedSession = serde_json::from_str(json).expect("deserialize");
        let session = persisted_into_session(persisted);

        // Then workflow state is None (graceful degradation).
        assert!(session.workflow().is_none());
    }

    // --- Test: Missing strategy blob means strategy_state is None ---

    #[rstest::rstest]
    fn missing_strategy_blob_means_no_strategy_state() {
        // Given a PersistedSession with no strategy_state blob.
        let persisted = PersistedSession {
            session_id: SessionId::new(),
            title: "No Strategy".to_owned(),
            updated_at: jiff::Timestamp::now(),
            history: vec![],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: HashMap::new(),
        };

        // When calling persisted_into_session.
        let session = persisted_into_session(persisted);

        // Then strategy_state is None.
        assert!(session.strategy_state().is_none());
    }
}
