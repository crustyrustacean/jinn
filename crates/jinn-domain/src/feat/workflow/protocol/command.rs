//! Workflow commands.

use serde::{Deserialize, Serialize};

use crate::feat::workflow::attached_workflow::{WorkflowConfig, WorkflowId, WorkflowTrigger};
use crate::protocol::{CommandMsg, SessionId};

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

/// Fire BeforeTurn workflows for a session (emitted by enqueue handler).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct FireBeforeTurn {
    pub session_id: SessionId,
}
