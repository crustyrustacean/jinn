//! Workflow runtime state types.

use std::collections::HashMap;
use std::sync::Arc;

use jinn_workflow::execution::WorkflowExecution;
use jinn_workflow::port::PortValues;
use tokio_util::sync::CancellationToken;

use crate::protocol::SessionId;

/// Unique identifier for a workflow execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct WorkflowId(String);

impl WorkflowId {
    /// Create a new unique workflow ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }
}

impl std::fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime state for a single executing workflow.
pub struct WorkflowState {
    /// Unique ID for this workflow execution.
    pub id: WorkflowId,
    /// The registered workflow name.
    pub name: String,
    /// Shared execution state - holds topology and node statuses.
    /// The engine writes status updates; the renderer reads snapshots.
    pub execution: Arc<WorkflowExecution>,
    /// Maps node name → session ID (for correlating StreamCompleted events).
    pub node_sessions: HashMap<String, SessionId>,
    /// Cancellation token for aborting execution.
    pub cancel: CancellationToken,
    /// Result after completion.
    pub result: Option<WorkflowResult>,
}

impl WorkflowState {
    /// Create a new workflow state with the given execution.
    pub fn new(name: String, execution: Arc<WorkflowExecution>) -> Self {
        Self {
            id: WorkflowId::new(),
            name,
            execution,
            node_sessions: HashMap::new(),
            cancel: CancellationToken::new(),
            result: None,
        }
    }
}

/// Result of a completed workflow execution.
#[derive(Debug)]
pub struct WorkflowResult {
    /// Final output values from all terminal nodes.
    pub outputs: HashMap<String, PortValues>,
    /// Whether the workflow completed successfully.
    pub success: bool,
}

/// Ephemeral runtime state for an attached workflow execution.
///
/// Lives in `AppState::workflow_executions`. Not persisted across restarts.
/// Keyed by `WorkflowId` (which is the same as `AttachedWorkflow::id`).
pub struct WorkflowExecutionState {
    /// Shared execution state — holds topology and node statuses.
    pub execution: Arc<WorkflowExecution>,
    /// Cancellation token for aborting execution.
    pub cancel: CancellationToken,
    /// The session that owns this attached workflow.
    pub session_id: SessionId,
    /// Maps node name → session ID for cloned sessions.
    pub node_sessions: HashMap<String, SessionId>,
}

impl std::fmt::Debug for WorkflowExecutionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowExecutionState")
            .field("session_id", &self.session_id)
            .field("node_sessions", &self.node_sessions)
            .finish_non_exhaustive()
    }
}

/// Map of loaded workflows with one active at a time.
///
/// Mirrors the [`SessionMap`](crate::common::session_map::SessionMap) pattern -
/// a map of workflow states with an active (focused) entry.
/// Unlike `SessionMap`, empty is a valid state (no workflows running).
#[derive(Default)]
pub struct WorkflowMap {
    workflows: HashMap<WorkflowId, WorkflowState>,
    active: Option<WorkflowId>,
}

impl std::fmt::Debug for WorkflowMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowMap")
            .field("count", &self.workflows.len())
            .field("active", &self.active)
            .finish()
    }
}

impl WorkflowMap {
    /// Returns a reference to the active workflow, if any.
    #[must_use]
    pub fn active(&self) -> Option<&WorkflowState> {
        self.active.as_ref().and_then(|id| self.workflows.get(id))
    }

    /// Returns a mutable reference to the active workflow, if any.
    pub fn active_mut(&mut self) -> Option<&mut WorkflowState> {
        let id = self.active.clone()?;
        self.workflows.get_mut(&id)
    }

    /// Insert a workflow and set it as active.
    pub fn insert(&mut self, state: WorkflowState) {
        self.active = Some(state.id.clone());
        self.workflows.insert(state.id.clone(), state);
    }

    /// Remove a workflow by ID. If it was active, active moves to any remaining.
    pub fn remove(&mut self, id: &WorkflowId) {
        self.workflows.remove(id);
        if self.active.as_ref() == Some(id) {
            self.active = self.workflows.keys().next().cloned();
        }
    }

    /// Set the active workflow by ID.
    pub fn set_active(&mut self, id: &WorkflowId) {
        if self.workflows.contains_key(id) {
            self.active = Some(id.clone());
        }
    }

    /// Get a workflow by ID.
    #[must_use]
    pub fn get(&self, id: &WorkflowId) -> Option<&WorkflowState> {
        self.workflows.get(id)
    }

    /// Get a mutable reference to a workflow by ID.
    pub fn get_mut(&mut self, id: &WorkflowId) -> Option<&mut WorkflowState> {
        self.workflows.get_mut(id)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::workflow::example::add_numbers;
    use std::sync::Arc;

    fn make_state(name: &str) -> WorkflowState {
        let graph = add_numbers::build_add_numbers();
        let execution = Arc::new(jinn_workflow::execution::WorkflowExecution::new(graph));
        WorkflowState::new(name.to_owned(), execution)
    }

    // --- WorkflowMap ---

    #[rstest::rstest]
    fn active_returns_none_when_empty() {
        // Given an empty map.
        let map = WorkflowMap::default();

        // Then active returns None.
        assert!(map.active().is_none());
    }

    #[rstest::rstest]
    fn active_mut_returns_none_when_empty() {
        // Given an empty map.
        let mut map = WorkflowMap::default();

        // Then active_mut returns None.
        assert!(map.active_mut().is_none());
    }

    #[rstest::rstest]
    fn insert_sets_active_and_stores_workflow() {
        // Given an empty map.
        let mut map = WorkflowMap::default();
        let state = make_state("test");
        let id = state.id.clone();

        // When inserting.
        map.insert(state);

        // Then it is active and retrievable.
        assert!(map.active().is_some());
        assert_eq!(map.active().unwrap().name, "test");
        assert!(map.get(&id).is_some());
        assert_eq!(map.get(&id).unwrap().name, "test");
    }

    #[rstest::rstest]
    fn active_mut_returns_inserted_workflow() {
        // Given a map with one workflow.
        let mut map = WorkflowMap::default();
        let state = make_state("test");
        let id = state.id.clone();
        map.insert(state);

        // When calling active_mut.
        let active = map.active_mut().expect("should have active");

        // Then it is mutable and correct.
        assert_eq!(active.id, id);
        active.name = "modified".to_owned();
        assert_eq!(map.active().unwrap().name, "modified");
    }

    #[rstest::rstest]
    fn insert_replaces_active() {
        // Given a map with one workflow.
        let mut map = WorkflowMap::default();
        let state1 = make_state("first");
        map.insert(state1);

        // When inserting a second.
        let state2 = make_state("second");
        map.insert(state2);

        // Then the second is active.
        assert_eq!(map.active().unwrap().name, "second");
    }

    #[rstest::rstest]
    fn remove_deletes_workflow() {
        // Given a map with one workflow.
        let mut map = WorkflowMap::default();
        let state = make_state("test");
        let id = state.id.clone();
        map.insert(state);

        // When removing.
        map.remove(&id);

        // Then it is gone and active moves to nothing.
        assert!(map.get(&id).is_none());
        assert!(map.active().is_none());
    }

    #[rstest::rstest]
    fn remove_active_shifts_to_remaining() {
        // Given a map with two workflows.
        let mut map = WorkflowMap::default();
        let state1 = make_state("first");
        let id1 = state1.id.clone();
        map.insert(state1);
        let state2 = make_state("second");
        let id2 = state2.id.clone();
        map.insert(state2);

        // Active is id2. When removing id2.
        map.remove(&id2);

        // Then id1 is still there.
        assert!(map.get(&id1).is_some());
        assert!(map.get(&id2).is_none());
    }

    #[rstest::rstest]
    fn remove_nonexistent_is_noop() {
        // Given a map with one workflow.
        let mut map = WorkflowMap::default();
        let state = make_state("test");
        let id = state.id.clone();
        map.insert(state);

        // When removing a different ID.
        let other_id = WorkflowId::new();
        map.remove(&other_id);

        // Then the original is unchanged.
        assert!(map.get(&id).is_some());
        assert_eq!(map.active().unwrap().id, id);
    }

    #[rstest::rstest]
    fn set_active_switches_focus() {
        // Given a map with two workflows.
        let mut map = WorkflowMap::default();
        let state1 = make_state("first");
        let id1 = state1.id.clone();
        map.insert(state1);
        let state2 = make_state("second");
        let _id2 = state2.id.clone();
        map.insert(state2);

        // Active is id2. When setting active to id1.
        map.set_active(&id1);

        // Then id1 is active.
        assert_eq!(map.active().unwrap().id, id1);
    }

    #[rstest::rstest]
    fn set_active_ignores_unknown_id() {
        // Given a map with one workflow.
        let mut map = WorkflowMap::default();
        let state = make_state("test");
        let id = state.id.clone();
        map.insert(state);

        // When setting active to unknown ID.
        let unknown = WorkflowId::new();
        map.set_active(&unknown);

        // Then active is unchanged.
        assert_eq!(map.active().unwrap().id, id);
    }

    #[rstest::rstest]
    fn get_mut_returns_mutable_reference() {
        // Given a map with one workflow.
        let mut map = WorkflowMap::default();
        let state = make_state("test");
        let id = state.id.clone();
        map.insert(state);

        // When getting mutable reference.
        let wf = map.get_mut(&id).expect("should exist");

        // Then it can be mutated.
        wf.name = "renamed".to_owned();
        assert_eq!(map.get(&id).unwrap().name, "renamed");
    }

    #[rstest::rstest]
    fn get_mut_returns_none_for_missing() {
        // Given an empty map.
        let mut map = WorkflowMap::default();

        // When getting mutable reference for any ID.
        let id = WorkflowId::new();

        // Then it returns None.
        assert!(map.get_mut(&id).is_none());
    }

    #[rstest::rstest]
    fn get_returns_none_for_missing() {
        // Given an empty map.
        let map = WorkflowMap::default();

        // When getting any ID.
        assert!(map.get(&WorkflowId::new()).is_none());
    }

    // --- WorkflowId ---

    #[rstest::rstest]
    fn workflow_id_default_creates_unique() {
        // Given two default IDs.
        let id1 = WorkflowId::default();
        let id2 = WorkflowId::default();

        // Then they differ.
        assert_ne!(id1, id2);
    }
}
