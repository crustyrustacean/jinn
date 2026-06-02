//! Rename workflow input intent handlers - enter, confirm, leave, and text editing.

use unicode_segmentation::UnicodeSegmentation;

use crate::common::app_state::{AppState, FocusScope, RenameWorkflowInputState};
use crate::feat::session_lifecycle::protocol::command::PersistSession;
use crate::feat::ui::sidebar::sessions::sorted_open_sessions;
use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;
use crate::protocol::{Command, IntentResult};

/// Opens the rename workflow input popup.
///
/// Pushes `FocusScope::RenameWorkflowInput` and seeds the input with the
/// currently selected workflow's label.
/// No-op if no entry is selected or if the selected entry is not a workflow.
pub fn handle_rename_workflow_enter(state: &mut AppState) -> IntentResult {
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return IntentResult::empty();
    };

    let entries = sorted_open_sessions(state);
    let Some(entry) = entries.get(index) else {
        return IntentResult::empty();
    };

    if entry.kind != SessionEntryKind::Workflow {
        return IntentResult::empty();
    }

    let session_id = &entry.id;

    let Some(session) = state.session.get(session_id) else {
        return IntentResult::empty();
    };

    let Some(workflow_id) = &entry.workflow_id else {
        return IntentResult::empty();
    };

    let aw = session
        .core
        .attached_workflows
        .iter()
        .find(|aw| &aw.id == workflow_id);

    let label = aw
        .map(|aw| aw.label_or_default().to_owned())
        .unwrap_or_default();

    let cursor_pos = label.len();

    state.frontend.rename_workflow_input = RenameWorkflowInputState {
        input: label,
        cursor_pos,
    };
    state
        .frontend
        .scope_stack
        .push(FocusScope::RenameWorkflowInput);
    IntentResult::empty()
}

/// Confirms the rename workflow input.
///
/// Validates the input (non-empty), updates the workflow label on the
/// owning session's `AttachedWorkflow`, pops the scope, clears state,
/// and emits a `PersistSession` command.
pub fn handle_rename_workflow_confirm(state: &mut AppState) -> IntentResult {
    let rename_input = &state.frontend.rename_workflow_input;
    let text = rename_input.input.trim().to_owned();

    // Validate: non-empty.
    if text.is_empty() {
        return IntentResult::empty();
    }

    // Resolve the selected entry.
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return IntentResult::empty();
    };
    let entries = sorted_open_sessions(state);
    let Some(entry) = entries.get(index) else {
        return IntentResult::empty();
    };

    let Some(workflow_id) = &entry.workflow_id else {
        return IntentResult::empty();
    };
    let session_id = entry.id.clone();
    // Update the workflow label.
    if let Some(session) = state.session.get_mut(&session_id) {
        if let Some(aw) = session
            .core
            .attached_workflows
            .iter_mut()
            .find(|aw| &aw.id == workflow_id)
        {
            aw.label = text;
        }
    }
    // Pop scope and clear state.
    state.frontend.scope_stack.pop();
    state.frontend.rename_workflow_input = RenameWorkflowInputState::default();

    IntentResult::with_commands(vec![Command::PersistSession(PersistSession { session_id })])
}

/// Cancels the rename workflow input popup.
///
/// Pops the scope and discards the input state.
pub fn handle_rename_workflow_leave(state: &mut AppState) -> IntentResult {
    state.frontend.scope_stack.pop();
    state.frontend.rename_workflow_input = RenameWorkflowInputState::default();
    IntentResult::empty()
}

/// Inserts a character at the cursor position.
pub fn handle_insert_char(state: &mut AppState, ch: char) -> IntentResult {
    let input = &mut state.frontend.rename_workflow_input;
    input.input.insert(input.cursor_pos, ch);
    input.cursor_pos += ch.len_utf8();
    IntentResult::empty()
}

/// Deletes the grapheme before the cursor.
pub fn handle_delete(state: &mut AppState) -> IntentResult {
    let input = &mut state.frontend.rename_workflow_input;
    if input.cursor_pos > 0 {
        let prev = input.input[..input.cursor_pos]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i);
        if let Some(prev_idx) = prev {
            input.input.drain(prev_idx..input.cursor_pos);
            input.cursor_pos = prev_idx;
        }
    }
    IntentResult::empty()
}

/// Deletes the grapheme at/after the cursor (forward delete).
pub fn handle_delete_forward(state: &mut AppState) -> IntentResult {
    let input = &mut state.frontend.rename_workflow_input;
    if input.cursor_pos < input.input.len() {
        let next_end = input.input[input.cursor_pos..]
            .grapheme_indices(true)
            .nth(1)
            .map_or(input.input.len(), |(i, _)| input.cursor_pos + i);
        input.input.drain(input.cursor_pos..next_end);
    }
    IntentResult::empty()
}

/// Moves the cursor one grapheme left.
pub fn handle_cursor_left(state: &mut AppState) -> IntentResult {
    let input = &mut state.frontend.rename_workflow_input;
    if input.cursor_pos > 0 {
        let prev = input.input[..input.cursor_pos]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i);
        if let Some(prev_idx) = prev {
            input.cursor_pos = prev_idx;
        }
    }
    IntentResult::empty()
}

/// Moves the cursor one grapheme right.
pub fn handle_cursor_right(state: &mut AppState) -> IntentResult {
    let input = &mut state.frontend.rename_workflow_input;
    if input.cursor_pos < input.input.len() {
        let next = input.input[input.cursor_pos..]
            .grapheme_indices(true)
            .nth(1)
            .map(|(i, _)| input.cursor_pos + i);
        match next {
            Some(next_idx) => input.cursor_pos = next_idx,
            None => input.cursor_pos = input.input.len(),
        }
    }
    IntentResult::empty()
}

/// Handles `PasteText` - bulk inserts pasted text at the cursor.
pub fn handle_paste(state: &mut AppState, text: &str) -> IntentResult {
    if text.is_empty() {
        return IntentResult::empty();
    }
    let input = &mut state.frontend.rename_workflow_input;
    input.input.insert_str(input.cursor_pos, text);
    input.cursor_pos += text.len();
    IntentResult::empty()
}
