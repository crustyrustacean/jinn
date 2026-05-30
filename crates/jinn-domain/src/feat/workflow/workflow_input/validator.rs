//! Workflow input validators.

use wherror::Error;

use crate::common::app_state::AppState;

/// Validates that the user can enter editing mode on the selected workflow node.
///
/// # Errors
///
/// Returns [`WorkflowEditNodeError`] if the conditions for editing are not met.
pub fn validate_workflow_edit_node(state: &AppState) -> Result<(), WorkflowEditNodeError> {
    let Some(workflow) = state.workflow.active() else {
        return Err(WorkflowEditNodeError::NoWorkflow);
    };

    let Some(node_name) = &state.frontend.workflow_ui.selected_node else {
        return Err(WorkflowEditNodeError::NoSelection);
    };

    // Check that the selected node is a source node (zero input ports).
    let snapshot = workflow.execution.snapshot();
    let structure = snapshot.structure();
    let is_source = structure
        .node_input_ports(node_name)
        .is_some_and(<[_]>::is_empty);

    if !is_source {
        return Err(WorkflowEditNodeError::NotSourceNode);
    }

    // Check that no node is currently running.
    let has_running = snapshot
        .statuses()
        .any(|(_, s)| s == jinn_workflow::engine::NodeStatus::Running);
    if has_running {
        return Err(WorkflowEditNodeError::WorkflowRunning);
    }

    Ok(())
}

/// Error type for workflow edit node validation.
#[derive(Debug, Error)]
pub enum WorkflowEditNodeError {
    /// No active workflow.
    #[error("no active workflow")]
    NoWorkflow,
    /// No node is selected.
    #[error("no node selected")]
    NoSelection,
    /// The selected node is not a source node.
    #[error("selected node is not a source node")]
    NotSourceNode,
    /// The workflow is currently running.
    #[error("workflow is running")]
    WorkflowRunning,
}
