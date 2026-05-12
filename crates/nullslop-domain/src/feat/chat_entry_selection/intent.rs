//! Chat entry selection intent handlers — navigate and pin entries.

use crate::common::app_state::AppState;
use crate::feat::context::protocol::command::PinChatEntry;
use crate::protocol::{Command, IntentResult, PinPosition};

use super::validator;

/// Selects the next chat entry in the active session.
pub fn handle_select_next(state: &mut AppState) -> IntentResult {
    validator::validate_chat_entry_select_next(state);
    state.active_session_mut().select_next_entry();
    IntentResult::empty()
}

/// Selects the previous chat entry in the active session.
pub fn handle_select_prev(state: &mut AppState) -> IntentResult {
    validator::validate_chat_entry_select_prev(state);
    state.active_session_mut().select_prev_entry();
    IntentResult::empty()
}

/// Pins the currently selected chat entry.
///
/// Returns a `PinChatEntry` command with `Relative` position.
pub fn handle_pin_selected(state: &mut AppState) -> IntentResult {
    if validator::validate_chat_entry_pin_selected(state).is_err() {
        return IntentResult::empty();
    }

    let session_id = state.session.active_session.clone();
    let Some(entry_id) = state.active_session().selected_entry_id().cloned() else {
        return IntentResult::empty();
    };

    IntentResult::with_commands(vec![Command::PinChatEntry {
        payload: PinChatEntry {
            session_id,
            entry_id,
            position: PinPosition::Relative,
        },
    }])
}

#[cfg(test)]
mod tests {
    use crate::common::app_state::AppState;
    use crate::feat::context::protocol::command::PinChatEntry;
    use crate::protocol::{ChatEntry, Command, PinPosition};

    use super::*;

    #[rstest::rstest]
    fn chat_entry_select_next_increments_index() {
        // Given a state with entries.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));

        // When handling select next.
        let result = handle_select_next(&mut state);

        // Then the first entry is selected.
        assert_eq!(state.active_session().selected_entry_index(), Some(0));
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn chat_entry_select_prev_decrements_index() {
        // Given a state with entries and selection at last.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        state.active_session_mut().select_prev_entry();

        // When handling select prev.
        let result = handle_select_prev(&mut state);

        // Then selection moved.
        assert_eq!(state.active_session().selected_entry_index(), Some(0));
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn chat_entry_pin_selected_returns_pin_command() {
        // Given a state with a selected entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When handling pin selected.
        let result = handle_pin_selected(&mut state);

        // Then a PinChatEntry command with Relative is returned.
        assert!(result.commands.iter().any(|c| {
            matches!(
                c,
                Command::PinChatEntry {
                    payload: PinChatEntry {
                        position: PinPosition::Relative,
                        ..
                    }
                }
            )
        }));
    }

    #[rstest::rstest]
    fn chat_entry_pin_selected_noop_with_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));

        // When handling pin selected.
        let result = handle_pin_selected(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }
}
