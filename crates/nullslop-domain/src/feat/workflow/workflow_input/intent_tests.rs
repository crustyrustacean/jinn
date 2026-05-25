//! Tests for workflow input intent handlers.

use crate::common::app_state::{AppState, FocusScope};
use crate::feat::intent::IntentHandler;
use crate::feat::workflow::workflow_state::WorkflowState;
use crate::protocol::Intent;
use nullslop_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use nullslop_workflow::node::code::CodeNode;
use nullslop_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

/// Helper: builds a minimal graph with one source node.
fn source_only_graph() -> WorkflowGraph {
    let source = CodeNode::new(
        "source".to_owned(),
        vec![],
        vec![PortDef::text("out")],
        |_inputs, _ctx| {
            Box::pin(async move {
                let mut out = PortValues::new();
                out.insert(
                    "out".to_owned(),
                    PortValue::Single(ScalarValue::Text("default".to_owned())),
                );
                Ok(out)
            })
        },
    );
    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("source".to_owned(), Box::new(source));
    builder.build().expect("graph should be valid")
}

/// Helper: inserts a workflow into state with Workflow as base scope.
fn insert_workflow_and_select_source(state: &mut AppState) {
    let execution = std::sync::Arc::new(nullslop_workflow::execution::WorkflowExecution::new(
        source_only_graph(),
    ));
    let workflow_state = WorkflowState::new("test".to_owned(), execution);
    state.workflow.insert(workflow_state);
    state.frontend.workflow_ui.selected_node = Some("source".to_owned());
    state.frontend.scope_stack.swap_base(FocusScope::Workflow);
}

#[test]
fn workflow_edit_node_pushes_workflow_input_scope() {
    // Given a workflow with a selected source node.
    let mut state = AppState::default();
    insert_workflow_and_select_source(&mut state);

    // When handling WorkflowEditNode.
    IntentHandler::handle(&Intent::WorkflowEditNode, &mut state);

    // Then the scope stack has WorkflowInput on top.
    assert_eq!(
        state.frontend.scope_stack.current(),
        &FocusScope::WorkflowInput
    );
    // And editing_node is set.
    assert_eq!(
        state.frontend.workflow_ui.editing_node,
        Some("source".to_owned())
    );
}

#[test]
fn workflow_input_submit_writes_output() {
    // Given state in WorkflowInput scope with buffer text "hello".
    let mut state = AppState::default();
    insert_workflow_and_select_source(&mut state);
    IntentHandler::handle(&Intent::WorkflowEditNode, &mut state);
    state.frontend.workflow_ui.input_buffer.insert_text("hello");

    // When handling WorkflowInputSubmit.
    IntentHandler::handle(&Intent::WorkflowInputSubmit, &mut state);

    // Then the execution snapshot shows "source" has outputs with the text.
    let workflow = state.workflow.active().expect("workflow exists");
    let snapshot = workflow.execution.snapshot();
    let node_state = snapshot.node_state("source").expect("node exists");
    let outputs = node_state.outputs.as_ref().expect("has outputs");
    assert_eq!(outputs.get_text("out").unwrap(), "hello");
}

#[test]
fn workflow_input_submit_pops_scope() {
    // Given state in WorkflowInput scope.
    let mut state = AppState::default();
    insert_workflow_and_select_source(&mut state);
    IntentHandler::handle(&Intent::WorkflowEditNode, &mut state);

    // When handling WorkflowInputSubmit.
    IntentHandler::handle(&Intent::WorkflowInputSubmit, &mut state);

    // Then editing_node is None and scope is back to Workflow.
    assert!(state.frontend.workflow_ui.editing_node.is_none());
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Workflow);
}

#[test]
fn workflow_input_cancel_discards_changes() {
    // Given state in WorkflowInput scope with buffer text.
    let mut state = AppState::default();
    insert_workflow_and_select_source(&mut state);
    IntentHandler::handle(&Intent::WorkflowEditNode, &mut state);
    state
        .frontend
        .workflow_ui
        .input_buffer
        .insert_text("typed stuff");

    // When handling WorkflowInputCancel.
    IntentHandler::handle(&Intent::WorkflowInputCancel, &mut state);

    // Then editing_node is None, buffer is empty, scope is Workflow.
    assert!(state.frontend.workflow_ui.editing_node.is_none());
    assert!(state.frontend.workflow_ui.input_buffer.is_empty());
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Workflow);
}

#[test]
fn workflow_input_insert_char_updates_buffer() {
    // Given WorkflowInput scope.
    let mut state = AppState::default();
    insert_workflow_and_select_source(&mut state);
    IntentHandler::handle(&Intent::WorkflowEditNode, &mut state);

    // When handling WorkflowInputInsertChar { ch: 'a' }.
    IntentHandler::handle(&Intent::WorkflowInputInsertChar { ch: 'a' }, &mut state);

    // Then the workflow buffer has "a" and chat input is empty.
    assert_eq!(state.frontend.workflow_ui.input_buffer.text(), "a");
    assert!(state.active_chat_input().is_empty());
}

#[test]
fn workflow_input_paste_text_updates_buffer() {
    // Given WorkflowInput scope.
    let mut state = AppState::default();
    insert_workflow_and_select_source(&mut state);
    IntentHandler::handle(&Intent::WorkflowEditNode, &mut state);

    // When handling WorkflowInputPasteText.
    IntentHandler::handle(
        &Intent::WorkflowInputPasteText {
            text: "hello world".to_owned(),
        },
        &mut state,
    );

    // Then the workflow buffer has "hello world" and chat input is empty.
    assert_eq!(
        state.frontend.workflow_ui.input_buffer.text(),
        "hello world"
    );
    assert!(state.active_chat_input().is_empty());
}

#[test]
fn workflow_input_delete_grapheme_removes_from_buffer() {
    // Given WorkflowInput scope with "ab" in buffer and cursor at end.
    let mut state = AppState::default();
    insert_workflow_and_select_source(&mut state);
    IntentHandler::handle(&Intent::WorkflowEditNode, &mut state);
    state.frontend.workflow_ui.input_buffer.insert_text("ab");

    // When handling WorkflowInputDeleteGrapheme.
    IntentHandler::handle(&Intent::WorkflowInputDeleteGrapheme, &mut state);

    // Then buffer is "a".
    assert_eq!(state.frontend.workflow_ui.input_buffer.text(), "a");
}

#[test]
fn workflow_input_cursor_left_moves_cursor() {
    // Given WorkflowInput scope with "ab" and cursor at end.
    let mut state = AppState::default();
    insert_workflow_and_select_source(&mut state);
    IntentHandler::handle(&Intent::WorkflowEditNode, &mut state);
    state.frontend.workflow_ui.input_buffer.insert_text("ab");

    // When handling WorkflowInputCursorLeft.
    IntentHandler::handle(&Intent::WorkflowInputCursorLeft, &mut state);

    // Then cursor is at position 1.
    assert_eq!(state.frontend.workflow_ui.input_buffer.cursor_pos(), 1);
}

#[test]
fn workflow_input_submit_transitions_status_to_pending() {
    // Given a source node in AwaitingInput status.
    let mut state = AppState::default();
    insert_workflow_and_select_source(&mut state);

    // Mark as AwaitingInput (simulating what handle_init_workflow does).
    let workflow = state.workflow.active().expect("workflow exists");
    workflow.execution.set_status(
        "source",
        nullslop_workflow::engine::NodeStatus::AwaitingInput,
    );

    // Enter edit mode and type text.
    IntentHandler::handle(&Intent::WorkflowEditNode, &mut state);
    state.frontend.workflow_ui.input_buffer.insert_text("hello");

    // When handling WorkflowInputSubmit.
    IntentHandler::handle(&Intent::WorkflowInputSubmit, &mut state);

    // Then the node status is now Pending.
    let workflow = state.workflow.active().expect("workflow exists");
    let snapshot = workflow.execution.snapshot();
    assert_eq!(
        snapshot.status_of("source"),
        Some(nullslop_workflow::engine::NodeStatus::Pending)
    );
}
