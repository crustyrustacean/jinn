//! Workflow runtime state types.

use std::collections::HashMap;

use nullslop_workflow::NodeStatus;
use nullslop_workflow::graph::WorkflowGraph;
use nullslop_workflow::port::PortValue;
use tokio_util::sync::CancellationToken;

use crate::protocol::SessionId;

/// Unique identifier for a workflow execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// A copy of the graph for rendering (the engine takes ownership of the original).
    pub graph_render_copy: Option<WorkflowGraph>,
    /// Per-node execution statuses.
    pub statuses: HashMap<String, NodeStatus>,
    /// Maps node name → session ID (for correlating StreamCompleted events).
    pub node_sessions: HashMap<String, SessionId>,
    /// Currently selected node in the UI.
    pub selected_node: Option<String>,
    /// Cancellation token for aborting execution.
    pub cancel: CancellationToken,
    /// Result after completion.
    pub result: Option<WorkflowResult>,
}

impl WorkflowState {
    /// Create a new workflow state with the given graph builder result.
    ///
    /// Note: The caller should clone the graph before passing it to the engine
    /// if they need it for rendering. The engine's `execute()` takes ownership.
    pub fn new(name: String, graph: WorkflowGraph) -> Self {
        let mut statuses = HashMap::new();
        for name in graph.node_names() {
            statuses.insert(name.to_owned(), NodeStatus::Pending);
        }
        let graph_render_copy = Some(graph);
        // Note: graph is consumed by engine on execute, so render_copy must be set
        // before calling engine. The engine takes ownership via execute().
        Self {
            id: WorkflowId::new(),
            name,
            graph_render_copy,
            statuses,
            node_sessions: HashMap::new(),
            selected_node: None,
            cancel: CancellationToken::new(),
            result: None,
        }
    }
}

/// Result of a completed workflow execution.
#[derive(Debug)]
pub struct WorkflowResult {
    /// Final output values from all terminal nodes.
    pub outputs: HashMap<String, nullslop_workflow::port::PortValues>,
    /// Whether the workflow completed successfully.
    pub success: bool,
}

use serde::{Deserialize, Serialize};

/// Map of loaded workflows with one active at a time.
///
/// Mirrors the [`SessionMap`] pattern — a non-empty map of workflow states
/// with an active (focused) entry.
pub struct WorkflowMap {
    workflows: HashMap<WorkflowId, WorkflowState>,
    active: Option<WorkflowId>,
}

impl Default for WorkflowMap {
    fn default() -> Self {
        Self {
            workflows: HashMap::new(),
            active: None,
        }
    }
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
