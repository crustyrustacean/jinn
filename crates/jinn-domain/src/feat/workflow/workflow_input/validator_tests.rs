//! Tests for workflow input validators.

use super::validator::{WorkflowEditNodeError, validate_workflow_edit_node};
use crate::common::app_state::AppState;
use crate::feat::workflow::workflow_state::WorkflowState;
use jinn_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use jinn_workflow::node::code::CodeNode;
use jinn_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

/// Helper: builds a minimal graph with one source node (no inputs, one text output).
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

/// Helper: builds a graph with a source and a non-source (has input port).
fn source_and_sink_graph() -> WorkflowGraph {
    let source = CodeNode::new(
        "source".to_owned(),
        vec![],
        vec![PortDef::text("out")],
        |_inputs, _ctx| {
            Box::pin(async move {
                let mut out = PortValues::new();
                out.insert(
                    "out".to_owned(),
                    PortValue::Single(ScalarValue::Text("data".to_owned())),
                );
                Ok(out)
            })
        },
    );
    let sink = CodeNode::new(
        "sink".to_owned(),
        vec![PortDef::text("in")],
        vec![],
        |_inputs, _ctx| Box::pin(async { Ok(PortValues::new()) }),
    );
    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("source".to_owned(), Box::new(source));
    builder.add_node("sink".to_owned(), Box::new(sink));
    builder
        .connect("source", "out", "sink", "in")
        .expect("connect");
    builder.build().expect("graph should be valid")
}

/// Helper: inserts a workflow into state and returns it.
fn insert_workflow(state: &mut AppState, graph: WorkflowGraph) {
    let execution = std::sync::Arc::new(jinn_workflow::execution::WorkflowExecution::new(graph));
    let workflow_state = WorkflowState::new("test".to_owned(), execution);
    state.workflow.insert(workflow_state);
}

#[test]
fn validate_edit_node_rejects_no_workflow() {
    // Given no active workflow.
    let state = AppState::default();

    // When validating.
    let result = validate_workflow_edit_node(&state);

    // Then returns NoWorkflow error.
    assert!(matches!(result, Err(WorkflowEditNodeError::NoWorkflow)));
}

#[test]
fn validate_edit_node_rejects_no_selection() {
    // Given an active workflow but no selected node.
    let mut state = AppState::default();
    insert_workflow(&mut state, source_only_graph());

    // When validating.
    let result = validate_workflow_edit_node(&state);

    // Then returns NoSelection error.
    assert!(matches!(result, Err(WorkflowEditNodeError::NoSelection)));
}

#[test]
fn validate_edit_node_rejects_non_source_node() {
    // Given a selected node that has input ports.
    let mut state = AppState::default();
    insert_workflow(&mut state, source_and_sink_graph());
    state.frontend.workflow_ui.selected_node = Some("sink".to_owned());

    // When validating.
    let result = validate_workflow_edit_node(&state);

    // Then returns NotSourceNode error.
    assert!(matches!(result, Err(WorkflowEditNodeError::NotSourceNode)));
}

#[test]
fn validate_edit_node_rejects_running_workflow() {
    // Given a source node selected but a node is Running.
    let mut state = AppState::default();
    insert_workflow(&mut state, source_only_graph());
    state.frontend.workflow_ui.selected_node = Some("source".to_owned());

    // Mark the source node as Running.
    let workflow = state.workflow.active().expect("workflow exists");
    workflow
        .execution
        .set_status("source", jinn_workflow::engine::NodeStatus::Running);

    // When validating.
    let result = validate_workflow_edit_node(&state);

    // Then returns WorkflowRunning error.
    assert!(matches!(
        result,
        Err(WorkflowEditNodeError::WorkflowRunning)
    ));
}

#[test]
fn validate_edit_node_accepts_source_node() {
    // Given an initialized workflow with a source node selected.
    let mut state = AppState::default();
    insert_workflow(&mut state, source_only_graph());
    state.frontend.workflow_ui.selected_node = Some("source".to_owned());

    // When validating.
    let result = validate_workflow_edit_node(&state);

    // Then validation passes.
    assert!(result.is_ok());
}
