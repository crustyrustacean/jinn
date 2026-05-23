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
    /// The user's prompt/topic text, passed to the graph builder.
    pub user_prompt: String,
}

/// Request to cancel a running workflow.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct CancelWorkflow {
    /// The workflow execution to cancel.
    pub workflow_id: WorkflowId,
}
