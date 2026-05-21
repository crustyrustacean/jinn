//! Queue a "Continue" user message to the session under the sidebar cursor.

use crate::common::app_state::AppState;
use crate::feat::chat_input::protocol::command::EnqueueUserMessage;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
use crate::protocol::{ChatEntry, Command, IntentResult};

/// Queues a "Continue" user message to the session under the sidebar cursor.
///
/// No session activation or scope change occurs. The message is enqueued
/// directly via the actor system.
pub fn handle_session_continue(state: &mut AppState) -> IntentResult {
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;

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

    let session_id = entry.id.clone();

    IntentResult::with_commands(vec![Command::EnqueueUserMessage(EnqueueUserMessage {
        session_id,
        entry: ChatEntry::user("Continue"),
    })])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::navigate_sidebar;
    use crate::feat::ui::sidebar::section_trait::SidebarIntent;
    use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
    use crate::protocol::Command;

    #[rstest::rstest]
    fn returns_enqueue_command_for_selected_session() {
        // Given a state with two sessions, sidebar focused on sessions section.
        let mut state = AppState::default();
        // Create a second session.
        let second_id = crate::protocol::SessionId::new();
        let mut second_session = crate::feat::session::chat_session::ChatSessionState::new();
        second_session.set_session_id(second_id.clone());
        state.session.insert(second_session);
        // Focus sidebar on sessions section.
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        // Navigate to select the second entry in the sorted list.
        navigate_sidebar(&SidebarIntent::MoveDown, &mut state);

        // Determine which session is at index 1 (the selected one).
        let sessions = sorted_open_sessions(&state);
        let selected_id = sessions[1].id.clone();
        let active_id_before = state.session.active_session_id().clone();

        // When handling session continue.
        let result = handle_session_continue(&mut state);

        // Then an EnqueueUserMessage command is returned.
        assert_eq!(result.commands.len(), 1);
        let cmd = &result.commands[0];
        let Command::EnqueueUserMessage(msg) = cmd else {
            panic!("expected EnqueueUserMessage, got {cmd:?}");
        };
        // And it targets the selected session.
        assert_eq!(msg.session_id, selected_id);
        // And the entry text is "Continue".
        assert_eq!(msg.entry.text(), "Continue");
        // And the active session is unchanged.
        assert_eq!(state.session.active_session_id(), &active_id_before);
    }

    #[rstest::rstest]
    fn noop_when_no_session_selected() {
        // Given a state with sidebar focused but no selected session.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        // No selection set.
        assert!(state.frontend.sessions_section.selected_index.is_none());

        // When handling session continue.
        let result = handle_session_continue(&mut state);

        // Then no commands are returned.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn noop_when_not_in_sessions_section() {
        // Given a state with sidebar focused on a different section.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarPersona);

        // When handling session continue.
        let result = handle_session_continue(&mut state);

        // Then no commands are returned.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn scope_stack_unchanged_after_continue() {
        // Given a state with sidebar focused on sessions section.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        navigate_sidebar(&SidebarIntent::MoveDown, &mut state);
        let scope_before = state.frontend.scope_stack.current().clone();

        // When handling session continue.
        let _result = handle_session_continue(&mut state);

        // Then the scope stack is unchanged.
        assert_eq!(
            state.frontend.scope_stack.current(),
            &scope_before,
            "scope should not change after session continue"
        );
    }
}
