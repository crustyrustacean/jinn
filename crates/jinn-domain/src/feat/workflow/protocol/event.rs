//! Workflow events.

use serde::{Deserialize, Serialize};

use crate::feat::workflow::attached_workflow::WorkflowId;
use crate::protocol::{EventMsg, SessionId};

/// An attached workflow was added to a session.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowAttached {
    pub session_id: SessionId,
    pub workflow_id: WorkflowId,
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
