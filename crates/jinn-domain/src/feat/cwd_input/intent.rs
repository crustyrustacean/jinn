//! Cwd input intent handlers - open, confirm, and leave.
//!
//! Skeleton stubs; filled in Phase 4.

use crate::common::app_state::AppState;
use crate::common::focus::FocusScope;
use crate::feat::cwd_input::resolve::{resolve_cwd_input, CwdResolution};
use crate::feat::cwd_input::state::CwdInputState;
use crate::feat::session_lifecycle::protocol::command::SetSessionCwd;
use crate::protocol::{Command, IntentResult};

/// Opens the cwd input popup.
///
/// Pushes `FocusScope::CwdInput` and seeds an empty [`CwdInputState`].
pub fn handle_cwd_input_enter(state: &mut AppState) -> IntentResult {
    state.frontend.cwd_input = CwdInputState::default();
    state.frontend.scope_stack.push(FocusScope::CwdInput);
    IntentResult::empty()
}

/// Confirms the cwd input.
///
// Resolves the typed path against the active session cwd; on success emits a
// [`SetSessionCwd`] command so the session actor applies the new cwd and
// broadcasts `SessionCwdChanged`. The event-driven scan actors pick up the
// new cwd automatically — no imperative scan commands are needed here.
// Then pops the scope and clears state. On failure (not a dir / empty) stays
// open with the inline error shown by the render footer.
pub fn handle_cwd_input_confirm(state: &mut AppState) -> IntentResult {
    let raw = state.frontend.cwd_input.text.input.trim().to_owned();
    let current_cwd = state.active_session().cwd().to_owned();
    let resolution = resolve_cwd_input(&raw, &current_cwd);

    if let CwdResolution::Ok(path) = resolution {
        let session_id = state.session.active_session_id().clone();
        let command = Command::SetSessionCwd(SetSessionCwd {
            session_id,
            cwd: path,
        });

        state.frontend.scope_stack.pop();
        state.frontend.cwd_input = CwdInputState::default();
        return IntentResult::with_commands(vec![command]);
    }

    IntentResult::empty()
}

/// Cancels the cwd input popup.
///
/// Pops the scope and discards the input state.
pub fn handle_cwd_input_leave(state: &mut AppState) -> IntentResult {
    state.frontend.scope_stack.pop();
    state.frontend.cwd_input = CwdInputState::default();
    IntentResult::empty()
}

/// Inserts a character at the cursor position.
pub fn handle_insert_char(state: &mut AppState, ch: char) -> IntentResult {
    state.frontend.cwd_input.text.insert_char(ch);
    IntentResult::empty()
}

/// Deletes the grapheme before the cursor.
pub fn handle_delete(state: &mut AppState) -> IntentResult {
    state.frontend.cwd_input.text.delete();
    IntentResult::empty()
}

/// Deletes the grapheme at/after the cursor (forward delete).
pub fn handle_delete_forward(state: &mut AppState) -> IntentResult {
    state.frontend.cwd_input.text.delete_forward();
    IntentResult::empty()
}

/// Moves the cursor one grapheme left.
pub fn handle_cursor_left(state: &mut AppState) -> IntentResult {
    state.frontend.cwd_input.text.cursor_left();
    IntentResult::empty()
}

/// Moves the cursor one grapheme right.
pub fn handle_cursor_right(state: &mut AppState) -> IntentResult {
    state.frontend.cwd_input.text.cursor_right();
    IntentResult::empty()
}

/// Handles `PasteText` - bulk inserts pasted text at the cursor.
pub fn handle_paste(state: &mut AppState, text: &str) -> IntentResult {
    state.frontend.cwd_input.text.paste(text);
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::common::app_state::FocusScope;

    #[rstest::rstest]
    fn enter_pushes_cwd_input_scope_and_seeds_default() {
        // Given default state (no cwd_input scope).
        let mut state = AppState::default();

        // When opening the cwd input popup.
        let result = handle_cwd_input_enter(&mut state);

        // Then CwdInput is the current scope and state is seeded default.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::CwdInput
        ));
        assert_eq!(state.frontend.cwd_input.text.input, "");
        assert_eq!(state.frontend.cwd_input.text.cursor_pos, 0);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn leave_pops_scope_and_clears_state() {
        // Given a state with cwd_input scope pushed + some typed text.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::CwdInput);
        state.frontend.cwd_input.text.input = "/some/path".to_owned();

        // When leaving.
        let result = handle_cwd_input_leave(&mut state);

        // Then the scope is popped and state is cleared.
        assert!(!matches!(
            state.frontend.scope_stack.current(),
            FocusScope::CwdInput
        ));
        assert_eq!(state.frontend.cwd_input.text.input, "");
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn confirm_valid_dir_returns_set_session_cwd_command() {
        // Given a tempdir and a session whose cwd is the tempdir's parent.
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path(); // target is itself a real dir
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_cwd(target.parent().unwrap().to_path_buf());
        // And the cwd_input has the tempdir's basename as text (relative).
        state.frontend.cwd_input.text.input =
            target.file_name().unwrap().to_string_lossy().to_string();

        // When confirming.
        let result = handle_cwd_input_confirm(&mut state);

        // Then exactly one SetSessionCwd command is returned for the active
        // session with the canonicalized target cwd. The session actor applies
        // the cwd asynchronously — the intent no longer mutates it directly.
        assert_eq!(result.commands.len(), 1);
        let expected = std::fs::canonicalize(target).unwrap();
        let cwd = match result.commands.first().unwrap() {
            Command::SetSessionCwd(p) => &p.cwd,
            other => panic!("expected SetSessionCwd, got {:?}", other),
        };
        assert_eq!(cwd, &expected);
    }

    #[rstest::rstest]
    fn confirm_valid_dir_pops_scope_and_clears_state() {
        // Given a tempdir and a session whose cwd is the tempdir's parent, with
        // the cwd input scope pushed and some typed text.
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path();
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_cwd(target.parent().unwrap().to_path_buf());
        state.frontend.scope_stack.push(FocusScope::CwdInput);
        state.frontend.cwd_input.text.input =
            target.file_name().unwrap().to_string_lossy().to_string();

        // When confirming.
        let _ = handle_cwd_input_confirm(&mut state);

        // Then the scope was popped and the input state was cleared.
        assert!(!matches!(
            state.frontend.scope_stack.current(),
            FocusScope::CwdInput
        ));
        assert_eq!(state.frontend.cwd_input.text.input, "");
    }

    #[rstest::rstest]
    fn confirm_nonexistent_path_stays_open_unchanged() {
        // Given a state with cwd_input scope pushed + a bad path.
        let mut state = AppState::default();
        let original_cwd = state.active_session().cwd().to_owned();
        state.frontend.scope_stack.push(FocusScope::CwdInput);
        state.frontend.cwd_input.text.input = "/this/does/not/exist".to_owned();

        // When confirming.
        let result = handle_cwd_input_confirm(&mut state);

        // Then nothing changed: scope intact, cwd unchanged, input intact.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::CwdInput
        ));
        assert_eq!(state.active_session().cwd(), original_cwd);
        assert_eq!(state.frontend.cwd_input.text.input, "/this/does/not/exist");
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn confirm_empty_input_is_noop() {
        // Given a state with cwd_input scope pushed + empty input.
        let mut state = AppState::default();
        let original_cwd = state.active_session().cwd().to_owned();
        state.frontend.scope_stack.push(FocusScope::CwdInput);

        // When confirming with empty input.
        let result = handle_cwd_input_confirm(&mut state);

        // Then nothing changed.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::CwdInput
        ));
        assert_eq!(state.active_session().cwd(), original_cwd);
        assert!(result.commands.is_empty());
    }
}
