//! Workflow commands.

use serde::{Deserialize, Serialize};

use crate::feat::workflow::workflow_state::WorkflowId;
use crate::protocol::CommandMsg;

/// Request to start a named workflow.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct StartWorkflow {
    /// The registered workflow name.
    pub name: String,
    /// A unique ID for this workflow execution.
    pub workflow_id: WorkflowId,
}

/// Request to cancel a running workflow.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct CancelWorkflow {
    /// The workflow execution to cancel.
    pub workflow_id: WorkflowId,
}

/// Request to re-run a workflow from a specific node.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct RerunFromNode {
    /// The workflow execution to re-run.
    pub workflow_id: WorkflowId,
    /// The node name to start re-execution from.
    pub node_name: String,
}

/// Request to load (initialize) a named workflow without executing it.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct InitWorkflow {
    /// The registered workflow name.
    pub name: String,
    /// A unique ID for this workflow execution.
    pub workflow_id: WorkflowId,
}

/// Request to load workflow picker entries from the registry.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct LoadWorkflowPickerEntries;
