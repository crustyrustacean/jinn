//! Workflow events.

use serde::{Deserialize, Serialize};

use crate::feat::workflow::workflow_state::WorkflowId;
use crate::protocol::EventMsg;

/// A workflow has been loaded (initialized) but not yet started.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowInitialized {
    /// The workflow execution ID.
    pub workflow_id: WorkflowId,
    /// The registered name of the workflow.
    pub name: String,
}

/// A workflow execution has started.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowStarted {
    /// The workflow execution ID.
    pub workflow_id: WorkflowId,
    /// The registered name of the workflow.
    pub name: String,
}

/// A workflow execution has completed (successfully or with an error).
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowCompleted {
    /// The workflow execution ID.
    pub workflow_id: WorkflowId,
    /// Whether the workflow completed successfully.
    pub success: bool,
}

/// A node in a workflow changed status.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowNodeStatusChanged {
    /// The workflow execution ID.
    pub workflow_id: WorkflowId,
    /// The node name.
    pub node_name: String,
    /// The new node status.
    pub status: String,
}

// --- Attached workflow events ---

use crate::protocol::SessionId;
use super::super::attached_workflow::WorkflowConfig;

/// An attached workflow was added to a session.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowAttached {
    pub session_id: SessionId,
    pub workflow_id: WorkflowId,
    pub config: WorkflowConfig,
}

/// An attached workflow was removed from a session.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowDetached {
    pub session_id: SessionId,
    pub workflow_id: WorkflowId,
}

/// An attached workflow was toggled on/off.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowToggled {
    pub session_id: SessionId,
    pub workflow_id: WorkflowId,
    pub enabled: bool,
}

/// An attached workflow completed execution.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct AttachedWorkflowCompleted {
    pub session_id: SessionId,
    pub workflow_id: WorkflowId,
    pub success: bool,
}

