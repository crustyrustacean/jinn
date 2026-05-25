//! Workflow input intent handlers.
//!
//! Handles entering editing mode, submitting edits, canceling, and all
//! character/cursor editing operations on the workflow input buffer.

use crate::common::app_state::{AppState, FocusScope};
use crate::protocol::IntentResult;

use super::validator;

/// Enters editing mode on the selected workflow source node.
///
/// Validates the edit is possible, pre-fills the buffer from existing output,
/// sets `editing_node`, and pushes `WorkflowInput` onto the scope stack.
pub fn handle_workflow_edit_node(state: &mut AppState) -> IntentResult {
    if validator::validate_workflow_edit_node(state).is_err() {
        return IntentResult::empty();
    }

    let Some(node_name) = &state.frontend.workflow_ui.selected_node else {
        return IntentResult::empty();
    };
    let node_name = node_name.clone();

    // Pre-fill buffer from existing output, if any.
    if let Some(workflow) = state.workflow.active() {
        let snapshot = workflow.execution.snapshot();
        if let Some(node_state) = snapshot.node_state(&node_name)
            && let Some(outputs) = &node_state.outputs
        {
            // Find the first text output port value.
            for (_, value) in outputs.iter() {
                if let nullslop_workflow::port::PortValue::Single(
                    nullslop_workflow::port::ScalarValue::Text(s),
                ) = value
                {
                    state
                        .frontend
                        .workflow_ui
                        .input_buffer
                        .replace_all(s.clone());
                    break;
                }
            }
        }
    }

    state.frontend.workflow_ui.editing_node = Some(node_name);
    state.frontend.scope_stack.push(FocusScope::WorkflowInput);

    IntentResult::empty()
}

/// Submits the workflow input buffer — writes the text to the source node's
/// output and exits edit mode.
pub fn handle_workflow_input_submit(state: &mut AppState) -> IntentResult {
    let Some(node_name) = state.frontend.workflow_ui.editing_node.clone() else {
        return IntentResult::empty();
    };

    let text = state.frontend.workflow_ui.input_buffer.text().to_owned();

    // Write the text to the source node's output.
    let Some(workflow) = state.workflow.active() else {
        return IntentResult::empty();
    };

    // Find the first text output port name from the structure.
    let snapshot = workflow.execution.snapshot();
    let structure = snapshot.structure();
    let output_port_name = structure
        .node_output_ports(&node_name)
        .and_then(|ports| {
            ports
                .iter()
                .find(|p| {
                    matches!(
                        p.value_type,
                        nullslop_workflow::port::PortType::Single(
                            nullslop_workflow::port::ScalarType::Text
                        )
                    )
                })
                .map(|p| p.name.clone())
        })
        .unwrap_or_else(|| "out".to_owned());

    let mut outputs = nullslop_workflow::port::PortValues::new();
    outputs.insert(
        output_port_name,
        nullslop_workflow::port::PortValue::Single(
            nullslop_workflow::port::ScalarValue::Text(text),
        ),
    );

    workflow.execution.set_node_outputs(&node_name, outputs);
    workflow
        .execution
        .set_status(&node_name, nullslop_workflow::engine::NodeStatus::Pending);

    // Clean up editing state.
    state.frontend.workflow_ui.input_buffer.reset();
    state.frontend.workflow_ui.editing_node = None;
    state.frontend.scope_stack.pop();

    IntentResult::empty()
}

/// Cancels workflow input editing — discards the buffer and exits edit mode.
pub fn handle_workflow_input_cancel(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.reset();
    state.frontend.workflow_ui.editing_node = None;
    state.frontend.scope_stack.pop();

    IntentResult::empty()
}

/// Inserts a character into the workflow input buffer.
pub fn handle_workflow_input_insert_char(ch: char, state: &mut AppState) -> IntentResult {
    state
        .frontend
        .workflow_ui
        .input_buffer
        .insert_grapheme_at_cursor(ch);
    IntentResult::empty()
}

/// Deletes the grapheme before the cursor in the workflow input buffer.
pub fn handle_workflow_input_delete_grapheme(state: &mut AppState) -> IntentResult {
    state
        .frontend
        .workflow_ui
        .input_buffer
        .delete_grapheme_before_cursor();
    IntentResult::empty()
}

/// Deletes the grapheme after the cursor (forward delete) in the workflow input buffer.
pub fn handle_workflow_input_delete_grapheme_forward(state: &mut AppState) -> IntentResult {
    state
        .frontend
        .workflow_ui
        .input_buffer
        .delete_grapheme_after_cursor();
    IntentResult::empty()
}

/// Pastes text into the workflow input buffer.
pub fn handle_workflow_input_paste_text(text: &str, state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.insert_text(text);
    IntentResult::empty()
}

/// Moves the cursor left in the workflow input buffer.
pub fn handle_workflow_input_cursor_left(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.move_cursor_left();
    IntentResult::empty()
}

/// Moves the cursor right in the workflow input buffer.
pub fn handle_workflow_input_cursor_right(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.move_cursor_right();
    IntentResult::empty()
}

/// Moves the cursor to the start of the workflow input buffer.
pub fn handle_workflow_input_cursor_to_start(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.move_cursor_to_start();
    IntentResult::empty()
}

/// Moves the cursor to the end of the workflow input buffer.
pub fn handle_workflow_input_cursor_to_end(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.move_cursor_to_end();
    IntentResult::empty()
}

/// Moves the cursor one word left in the workflow input buffer.
pub fn handle_workflow_input_cursor_word_left(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.move_cursor_word_left();
    IntentResult::empty()
}

/// Moves the cursor one word right in the workflow input buffer.
pub fn handle_workflow_input_cursor_word_right(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.move_cursor_word_right();
    IntentResult::empty()
}

/// Moves the cursor up one visual line in the workflow input buffer.
pub fn handle_workflow_input_cursor_up(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.move_cursor_up();
    IntentResult::empty()
}

/// Moves the cursor down one visual line in the workflow input buffer.
pub fn handle_workflow_input_cursor_down(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.input_buffer.move_cursor_down();
    IntentResult::empty()
}
