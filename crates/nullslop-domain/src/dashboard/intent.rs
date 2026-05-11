//! Dashboard intent handlers — selection navigation.
//!
//! Handles the four dashboard navigation intents: move selection down, up,
//! to the first entry, and to the last entry. All are infallible and return
//! no commands.

use crate::component::AppState;
use crate::protocol::IntentResult;

/// Move the dashboard selection to the next entry.
pub fn handle_select_down(state: &mut AppState) -> IntentResult {
    state.frontend.dashboard.select_next();
    IntentResult::empty()
}

/// Move the dashboard selection to the previous entry.
pub fn handle_select_up(state: &mut AppState) -> IntentResult {
    state.frontend.dashboard.select_prev();
    IntentResult::empty()
}

/// Move the dashboard selection to the first entry.
pub fn handle_select_first(state: &mut AppState) -> IntentResult {
    state.frontend.dashboard.select_first();
    IntentResult::empty()
}

/// Move the dashboard selection to the last entry.
pub fn handle_select_last(state: &mut AppState) -> IntentResult {
    state.frontend.dashboard.select_last();
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn select_down_moves_selection() {
        // Given a state with dashboard entries.
        let mut state = AppState::default();
        state.frontend.dashboard.mark_starting("echo", None);
        state.frontend.dashboard.mark_starting("llm", None);

        // When handling select down.
        let result = handle_select_down(&mut state);

        // Then the selection has moved.
        assert_eq!(state.frontend.dashboard.selected_index(), 1);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn select_up_moves_selection() {
        // Given a state with dashboard entries at index 1.
        let mut state = AppState::default();
        state.frontend.dashboard.mark_starting("echo", None);
        state.frontend.dashboard.mark_starting("llm", None);
        state.frontend.dashboard.select_next();

        // When handling select up.
        let result = handle_select_up(&mut state);

        // Then the selection is at 0.
        assert_eq!(state.frontend.dashboard.selected_index(), 0);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn select_first_moves_to_first() {
        // Given a state with entries at last index.
        let mut state = AppState::default();
        state.frontend.dashboard.mark_starting("echo", None);
        state.frontend.dashboard.mark_starting("llm", None);
        state.frontend.dashboard.mark_starting("ctx", None);
        state.frontend.dashboard.select_next();
        state.frontend.dashboard.select_next();

        // When handling select first.
        let result = handle_select_first(&mut state);

        // Then the selection is at 0.
        assert_eq!(state.frontend.dashboard.selected_index(), 0);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn select_last_moves_to_last() {
        // Given a state with 3 dashboard entries.
        let mut state = AppState::default();
        state.frontend.dashboard.mark_starting("echo", None);
        state.frontend.dashboard.mark_starting("llm", None);
        state.frontend.dashboard.mark_starting("ctx", None);

        // When handling select last.
        let result = handle_select_last(&mut state);

        // Then the selection is at the last index.
        assert_eq!(state.frontend.dashboard.selected_index(), 2);
        assert!(result.commands.is_empty());
    }
}
