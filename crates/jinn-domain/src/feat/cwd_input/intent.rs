//! Cwd input intent handlers - open, confirm, and leave.
//!
//! Skeleton stubs; filled in Phase 4.

use crate::common::app_state::AppState;
use crate::common::focus::FocusScope;
use crate::feat::cwd_input::resolve::{resolve_cwd_input, CwdResolution};
use crate::feat::cwd_input::state::CwdInputState;
use crate::protocol::IntentResult;

/// Opens the cwd input popup.
///
/// Pushes `FocusScope::CwdInput` and seeds an empty [`CwdInputState`].
pub fn handle_cwd_input_enter(state: &mut AppState) -> IntentResult {
    state.frontend.cwd_input = CwdInputState::default();
    state.frontend.scope_stack.push(FocusScope::CwdInput);
    IntentResult::empty()
}

/// Confirms the cwd input.
//
// Resolves the typed path against the active session cwd; on success sets the
// session cwd and rescans context files inline, then pops the scope and clears
// state. On failure (not a dir / empty) stays open with the inline error shown
// by the render footer.
pub fn handle_cwd_input_confirm(state: &mut AppState) -> IntentResult {
    let raw = state.frontend.cwd_input.text.input.trim().to_owned();
    let current_cwd = state.active_session().cwd().to_owned();
    let resolution = resolve_cwd_input(&raw, &current_cwd);

    if let CwdResolution::Ok(path) = resolution {
        state.active_session_mut().set_cwd(path.clone());
        state.context.context_files =
            crate::feat::context::env_context::load_project_context_files_sync(&path);
        state.frontend.scope_stack.pop();
        state.frontend.cwd_input = CwdInputState::default();
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
    fn confirm_valid_dir_sets_cwd_rescans_context_pops_and_clears() {
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

        // Then the session cwd is now the tempdir (canonicalized).
        let expected = std::fs::canonicalize(target).unwrap();
        assert_eq!(state.active_session().cwd(), expected);
        // And context files were rescanned (non-empty vector, even if no
        // AGENTS.md present the vector is well-formed).
        let _ = &state.context.context_files;
        // And scope was popped + state cleared.
        assert!(!matches!(
            state.frontend.scope_stack.current(),
            FocusScope::CwdInput
        ));
        assert_eq!(state.frontend.cwd_input.text.input, "");
        assert!(result.commands.is_empty());
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
