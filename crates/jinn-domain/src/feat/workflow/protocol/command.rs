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

// --- Attached workflow commands ---


use crate::protocol::SessionId;
use super::super::attached_workflow::{WorkflowConfig, WorkflowTrigger};

/// Attach a workflow to a session.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct AttachWorkflow {
    pub session_id: SessionId,
    pub config: WorkflowConfig,
    pub trigger: WorkflowTrigger,
}

/// Detach a workflow from a session.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct DetachWorkflow {
    pub session_id: SessionId,
    pub workflow_id: WorkflowId,
}

/// Toggle an attached workflow on/off.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct ToggleWorkflow {
    pub session_id: SessionId,
    pub workflow_id: WorkflowId,
}

/// Manually trigger an attached workflow.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct TriggerWorkflow {
    pub session_id: SessionId,
    pub workflow_id: WorkflowId,
}

/// Emitted by enqueue handler when a BeforeTurn workflow defers the user message.
/// The controller picks this up, fires the BeforeTurn, then re-enqueues the merged text.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct FireBeforeTurn {
    pub session_id: SessionId,
}

