//! Workflow events.

use serde::{Deserialize, Serialize};

use crate::feat::workflow::workflow_state::WorkflowId;
use crate::protocol::EventMsg;

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
