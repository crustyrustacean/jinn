//! Activates the session or workflow under the cursor.

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;

/// Activates the session or workflow under the cursor.
///
/// Called when the user presses Enter in the sessions section.
/// Uses `swap_base` to replace the entire scope stack, effectively
/// closing the sidebar and switching to the target view.
/// - For session entries: swaps to Normal (chat view).
/// - For workflow entries: swaps to Workflow (graph view).
pub fn handle_session_activate(state: &mut AppState) {
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
    use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;

    if !matches!(
        state.frontend.scope_stack.sidebar_section(),
        Some(SidebarSectionId::Sessions)
    ) {
        return;
    }
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return;
    };
    let sessions = sorted_open_sessions(state);
    let Some(entry) = sessions.get(index) else {
        return;
    };

    match entry.kind {
        SessionEntryKind::Session => {
            state.session.set_active(entry.id.clone());
            state.frontend.scope_stack.swap_base(FocusScope::Normal);
        }
        SessionEntryKind::Workflow => {
            let Some(wf_id) = &entry.workflow_id else {
                return;
            };
            if state.workflow.get(wf_id).is_some() {
                state.session.set_active(entry.id.clone());
                state.workflow.set_active(wf_id);
                state.frontend.scope_stack.swap_base(FocusScope::Workflow);
            }

        }
    }

    // Clear preview after activation decision.
    state.frontend.sessions_section.previewed_workflow_id = None;
}


#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
    use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;
    use crate::feat::workflow::attached_workflow::{AttachedWorkflow, AttachedWorkflowState, WorkflowConfig, WorkflowTrigger};
    use crate::feat::workflow::workflow_state::{WorkflowId, WorkflowState};
    use crate::protocol::SessionId;

    fn test_graph() -> jinn_workflow::graph::WorkflowGraph {
        use jinn_workflow::node::code::CodeNode;
        use jinn_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};
        let source = CodeNode::new(
            "source".to_owned(),
            vec![],
            vec![PortDef::text("out")],
            |_inputs, _ctx| {
                Box::pin(async move {
                    let mut out = PortValues::new();
                    out.insert("out".to_owned(), PortValue::Single(ScalarValue::Text("data".to_owned())));
                    Ok(out)
                })
            },
        );
        let mut builder = jinn_workflow::graph::WorkflowGraphBuilder::new();
        builder.add_node("source".to_owned(), Box::new(source));
        builder.build().expect("test graph should build")
    }

    #[rstest::rstest]
    fn activating_session_swaps_base_to_normal() {
        // Given a state with Workflow base and sidebar sessions overlay.
        let mut state = AppState::default();
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        // Set up a second session to activate.
        let second_id = SessionId::new();
        let mut second_session = crate::feat::session::chat_session::ChatSessionState::new();
        second_session.set_session_id(second_id.clone());
        state.session.insert(second_session);
        // Position cursor on the second session.
        state.frontend.sessions_section.selected_index = Some(1);

        // When activating.
        handle_session_activate(&mut state);

        // Then base is Normal (not Workflow), sidebar overlay is gone.
        assert_eq!(
            state.frontend.scope_stack.current(),
            &FocusScope::Normal,
            "base should be Normal after activating session"
        );
        assert_eq!(state.frontend.scope_stack.len(), 1, "sidebar overlay should be gone");
    }

    #[rstest::rstest]
    fn activating_workflow_swaps_base_to_workflow() {
        // Given a state with a workflow in WorkflowMap and sidebar overlay.
        let mut state = AppState::default();
        let wf_id = WorkflowId::new();
        let execution = std::sync::Arc::new(
            jinn_workflow::execution::WorkflowExecution::new(
                test_graph(),
            ),
        );

        let wf_state = WorkflowState::new("test".into(), execution);
        state.workflow.insert(WorkflowState {
            id: wf_id.clone(),
            ..wf_state
        });
        // Add AttachedWorkflow to session.
        let session_id = state.session.active_session_id().clone();
        state.session.get_mut(&session_id).unwrap().core.attached_workflows.push(
            AttachedWorkflow {
                id: wf_id.clone(),
                config: WorkflowConfig::Custom(serde_json::json!({})),
                label: "Custom".to_owned(),
                trigger: WorkflowTrigger::Manual,
                enabled: true,
                state: AttachedWorkflowState::Ready,
            },
        );
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        // Position cursor on the workflow entry.
        state.frontend.sessions_section.selected_index = Some(1);

        // When activating.
        handle_session_activate(&mut state);

        // Then base is Workflow, sidebar overlay is gone.
        assert_eq!(
            state.frontend.scope_stack.current(),
            &FocusScope::Workflow,
            "base should be Workflow after activating workflow"
        );
        assert_eq!(state.frontend.scope_stack.len(), 1, "sidebar overlay should be gone");
    }

    #[rstest::rstest]
    fn activating_workflow_also_activates_owning_session() {
        // Given a state with two sessions, workflow on the second (inactive) session.
        let mut state = AppState::default();
        let first_session_id = state.session.active_session_id().clone();

        // Create a second session.
        let second_id = SessionId::new();
        let mut second_session = crate::feat::session::chat_session::ChatSessionState::new();
        second_session.set_session_id(second_id.clone());
        state.session.insert(second_session);

        // Add a workflow to the second session.
        let wf_id = WorkflowId::new();
        let execution = std::sync::Arc::new(
            jinn_workflow::execution::WorkflowExecution::new(
                test_graph(),
            ),
        );
        let wf_state = WorkflowState::new("test".into(), execution);
        state.workflow.insert(WorkflowState {
            id: wf_id.clone(),
            ..wf_state
        });
        state.session.get_mut(&second_id).unwrap().core.attached_workflows.push(
            AttachedWorkflow {
                id: wf_id.clone(),
                config: WorkflowConfig::Custom(serde_json::json!({})),
                label: "Custom".to_owned(),
                trigger: WorkflowTrigger::Manual,
                enabled: true,
                state: AttachedWorkflowState::Ready,
            },
        );
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        // Position cursor on the workflow entry (index 1: second_session[0], workflow[1], first_session[2]).
        state.frontend.sessions_section.selected_index = Some(1);

        // Verify the first session is active before activation.
        assert_eq!(
            state.session.active_session_id(),
            &first_session_id,
            "first session should be active before activation"
        );

        // When activating.
        handle_session_activate(&mut state);

        // Then the owning (second) session is now active.
        assert_eq!(
            state.session.active_session_id(),
            &second_id,
            "activating a workflow should also activate its owning session"
        );
    }
    }
