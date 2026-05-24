//! Workflow runtime state types.

use std::collections::HashMap;
use std::sync::Arc;

use nullslop_workflow::execution::WorkflowExecution;
use nullslop_workflow::port::PortValues;
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
    /// Shared execution state — holds topology and node statuses.
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

/// Map of loaded workflows with one active at a time.
///
/// Mirrors the [`SessionMap`](crate::common::session_map::SessionMap) pattern —
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
