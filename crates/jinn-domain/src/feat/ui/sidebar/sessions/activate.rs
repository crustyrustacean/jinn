//! Activates the session or workflow under the cursor.

use crate::common::app_state::AppState;

use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
use crate::protocol::IntentResult;

/// Activates the session or workflow under the cursor.
///
/// Called when the user presses Enter in the sessions section.
/// Uses `swap_base` to replace the entire scope stack, effectively
/// closing the sidebar and switching to the target view.
/// - For session entries: swaps to Normal (chat view). No re-scan commands
///   are emitted: each session's discovered skills/prompts/context-files
///   are ephemeral and persist across activation changes, and were
///   hydrated when the session was created/loaded.
/// - For workflow entries: swaps to Workflow (graph view).
pub fn handle_session_activate(state: &mut AppState) -> IntentResult {
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
    use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;

    if !matches!(
        state.frontend.scope_stack.sidebar_section(),
        Some(SidebarSectionId::Sessions)
    ) {
        return IntentResult::empty();
    }
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return IntentResult::empty();
    };
    let sessions = sorted_open_sessions(state);
    let Some(entry) = sessions.get(index) else {
        return IntentResult::empty();
    };

    match entry.kind {
        SessionEntryKind::Session => {
            state.session.set_active(entry.id.clone());
            state.frontend.scope_stack.swap_base(FocusScope::Normal);
            IntentResult::empty()
        }
        SessionEntryKind::Plugin { .. } => {
            // Workflow entries are informational only; activating them is a no-op.
            IntentResult::empty()
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::focus::FocusScope;

    use crate::protocol::SessionId;

    /// Two sessions exist in state; cursor points at the second-inserted session.
    fn state_with_two_sessions_cursor_on_second() -> (AppState, SessionId) {
        let mut state = AppState::default();
        let _first = state.session.active_session_id().clone();
        let second_session = crate::feat::session::chat_session::ChatSessionState::default();
        let second = second_session.session_id().clone();
        state.session.insert(second_session);
        // Cursor points at the second session in sorted order.
        let sessions = sorted_open_sessions(&state);
        let target_idx = sessions
            .iter()
            .position(|e| e.id == second)
            .expect("second session present");
        state.frontend.sessions_section.selected_index = Some(target_idx);
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        (state, second)
    }

    #[rstest::rstest]
    fn activate_session_switches_active_session_and_emits_no_commands() {
        // Given a sessions sidebar with cursor on a non-active session.
        let (mut state, expected_id) = state_with_two_sessions_cursor_on_second();

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then the active session is now the one under the cursor.
        assert_eq!(state.session.active_session_id(), &expected_id);
        // And no commands are emitted: each session's discovered
        // skills/prompts/context-files are ephemeral and were hydrated when the
        // session was created/loaded, so activation needs no re-scan.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn activate_session_with_no_cursor_emits_nothing() {
        // Given sessions sidebar but no selected index.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        // selected_index stays None.

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then no commands emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn activate_outside_sessions_section_emits_nothing() {
        // Given Normal scope (not sessions sidebar).
        let mut state = AppState::default();

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then no commands emitted.
        assert!(result.commands.is_empty());
    }
}
