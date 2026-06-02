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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::common::app_state::FocusScope;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::workflow::attached_workflow::{
        AttachedWorkflow, AttachedWorkflowState, ResultKind, WorkflowConfig, WorkflowTrigger,
    };
    use crate::protocol::ChatEntry;

    /// Create a state with a session that has an attached workflow.
    fn state_with_workflow(label: &str) -> (AppState, crate::protocol::SessionId) {
        let mut state = AppState::default();
        let session_id = state.session.active_session_id().clone();

        // Add an attached workflow to the active session.
        let mut aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: ResultKind::Assistant,
            },
            WorkflowTrigger::TurnEnd,
        );
        aw.label = label.to_owned();
        let wf_id = aw.id.clone();

        state.session_mut(&session_id).core.attached_workflows.push(aw);

        // Set up sidebar state: select the workflow entry.
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        // Compute entries and find the workflow entry index.
        let entries = sorted_open_sessions(&state);
        let wf_index = entries
            .iter()
            .position(|e| e.kind == SessionEntryKind::Workflow)
            .expect("should have a workflow entry");
        state.frontend.sessions_section.selected_index = Some(wf_index);

        (state, session_id)
    }

    #[rstest::rstest]
    fn enter_pushes_rename_workflow_input_scope() {
        // Given a state with a workflow.
        let (mut state, _sid) = state_with_workflow("My Workflow");

        // When handling rename workflow enter.
        let result = handle_rename_workflow_enter(&mut state);

        // Then RenameWorkflowInput is the current scope.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::RenameWorkflowInput
        ));
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn enter_seeds_input_with_current_label() {
        // Given a state with a workflow labeled "My Workflow".
        let (mut state, _sid) = state_with_workflow("My Workflow");

        // When handling rename workflow enter.
        let _result = handle_rename_workflow_enter(&mut state);

        // Then the input is seeded with the workflow label.
        assert_eq!(state.frontend.rename_workflow_input.input, "My Workflow");
        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 11);
    }

    #[rstest::rstest]
    fn enter_noop_when_no_selection() {
        // Given a state with no selection.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);

        // When handling rename workflow enter.
        let result = handle_rename_workflow_enter(&mut state);

        // Then scope is unchanged.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::SidebarSessions
        ));
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn enter_noop_for_session_entry() {
        // Given a state with a session but no workflow.
        let mut state = AppState::default();
        let session_id = state.session.active_session_id().clone();
        state
            .session_mut(&session_id)
            .push_entry(ChatEntry::user("hello"));
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        // Select the session entry (index 0).
        state.frontend.sessions_section.selected_index = Some(0);

        // When handling rename workflow enter.
        let result = handle_rename_workflow_enter(&mut state);

        // Then scope is unchanged (no workflow at that index).
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::SidebarSessions
        ));
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn confirm_updates_workflow_label() {
        // Given a state in RenameWorkflowInput scope with input "New Label".
        let (mut state, session_id) = state_with_workflow("Old Label");
        state.frontend.scope_stack.push(FocusScope::RenameWorkflowInput);
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "New Label".to_owned(),
            cursor_pos: 9,
        };

        // When handling rename workflow confirm.
        let result = handle_rename_workflow_confirm(&mut state);

        // Then the workflow label is updated.
        let session = state.session.get(&session_id).expect("session exists");
        let aw = session.core.attached_workflows.first().expect("has workflow");
        assert_eq!(aw.label, "New Label");
        // And scope is popped back.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::SidebarSessions
        ));
        // And input state is cleared.
        assert!(state.frontend.rename_workflow_input.input.is_empty());
        // And a PersistSession command is emitted.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::PersistSession(p) if p.session_id == session_id)),
            "expected PersistSession command"
        );
    }

    #[rstest::rstest]
    fn confirm_rejects_empty_input() {
        // Given a state with empty input.
        let (mut state, _sid) = state_with_workflow("Old");
        state.frontend.scope_stack.push(FocusScope::RenameWorkflowInput);
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: String::new(),
            cursor_pos: 0,
        };

        // When handling rename workflow confirm.
        let result = handle_rename_workflow_confirm(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
        // And scope is NOT popped.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::RenameWorkflowInput
        ));
    }

    #[rstest::rstest]
    fn leave_discards_changes() {
        // Given a state in RenameWorkflowInput scope.
        let (mut state, session_id) = state_with_workflow("Original");
        state.frontend.scope_stack.push(FocusScope::RenameWorkflowInput);
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Changed".to_owned(),
            cursor_pos: 7,
        };

        // When handling rename workflow leave.
        let result = handle_rename_workflow_leave(&mut state);

        // Then scope is popped back.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::SidebarSessions
        ));
        // And input state is cleared.
        assert!(state.frontend.rename_workflow_input.input.is_empty());
        // And workflow label is unchanged.
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(
            session.core.attached_workflows[0].label,
            "Original"
        );
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn insert_char_adds_character() {
        let mut state = AppState::default();
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Hello".to_owned(),
            cursor_pos: 5,
        };

        let _result = handle_insert_char(&mut state, '!');

        assert_eq!(state.frontend.rename_workflow_input.input, "Hello!");
        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 6);
    }

    #[rstest::rstest]
    fn delete_removes_grapheme_before_cursor() {
        let mut state = AppState::default();
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Hello".to_owned(),
            cursor_pos: 5,
        };

        let _result = handle_delete(&mut state);

        assert_eq!(state.frontend.rename_workflow_input.input, "Hell");
        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 4);
    }

    #[rstest::rstest]
    fn delete_forward_removes_grapheme_after_cursor() {
        let mut state = AppState::default();
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Hello".to_owned(),
            cursor_pos: 1,
        };

        let _result = handle_delete_forward(&mut state);

        assert_eq!(state.frontend.rename_workflow_input.input, "Hllo");
        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 1);
    }

    #[rstest::rstest]
    fn cursor_left_moves_back() {
        let mut state = AppState::default();
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Hi".to_owned(),
            cursor_pos: 2,
        };

        let _result = handle_cursor_left(&mut state);

        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 1);
    }

    #[rstest::rstest]
    fn cursor_right_moves_forward() {
        let mut state = AppState::default();
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Hi".to_owned(),
            cursor_pos: 0,
        };

        let _result = handle_cursor_right(&mut state);

        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 1);
    }

    // --- Boundary tests ---

    #[rstest::rstest]
    fn handle_delete_noop_at_position_zero() {
        let mut state = AppState::default();
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Hello".to_owned(),
            cursor_pos: 0,
        };

        let _result = handle_delete(&mut state);

        assert_eq!(state.frontend.rename_workflow_input.input, "Hello");
        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 0);
    }

    #[rstest::rstest]
    fn handle_delete_forward_noop_at_end() {
        let mut state = AppState::default();
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Hello".to_owned(),
            cursor_pos: 5,
        };

        let _result = handle_delete_forward(&mut state);

        assert_eq!(state.frontend.rename_workflow_input.input, "Hello");
        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 5);
    }

    #[rstest::rstest]
    fn handle_paste_inserts_text_and_advances_cursor() {
        let mut state = AppState::default();
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Hello".to_owned(),
            cursor_pos: 2,
        };

        let _result = handle_paste(&mut state, "XY");

        assert_eq!(state.frontend.rename_workflow_input.input, "HeXYllo");
        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 4);
    }

    #[rstest::rstest]
    fn handle_paste_noop_when_empty() {
        let mut state = AppState::default();
        state.frontend.rename_workflow_input = RenameWorkflowInputState {
            input: "Hello".to_owned(),
            cursor_pos: 2,
        };

        let _result = handle_paste(&mut state, "");

        assert_eq!(state.frontend.rename_workflow_input.input, "Hello");
        assert_eq!(state.frontend.rename_workflow_input.cursor_pos, 2);
    }

    #[rstest::rstest]
    fn routing_session_entry_opens_session_rename() {
        // Given a state with a session but no workflow, sidebar on sessions.
        let mut state = AppState::default();
        let session_id = state.session.active_session_id().clone();
        state
            .session_mut(&session_id)
            .set_title("My Session".to_owned());
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(0);

        // When handling rename workflow enter on a session entry.
        let result = handle_rename_workflow_enter(&mut state);

        // Then it is a noop (not a workflow entry).
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::SidebarSessions
        ));
        assert!(result.commands.is_empty());
    }
}