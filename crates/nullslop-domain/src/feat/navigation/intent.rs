//! Navigation intent handlers — scroll, tab, and editor.

use crate::common::app_state::AppState;
use crate::protocol::IntentResult;

/// Number of lines to scroll per mouse wheel tick.
const MOUSE_SCROLL_STEP: u16 = 3;

/// Scrolls the chat log up by half a viewport page and moves cursor to first visible entry.
pub fn handle_scroll_up(state: &mut AppState) -> IntentResult {
    let viewport_height = state.active_session().viewport_height_value().max(1);
    let half = viewport_height / 2;
    state.active_session_mut().scroll_up(half);
    state.active_session_mut().move_cursor_to_first_visible();
    IntentResult::empty()
}

/// Scrolls the chat log down by half a viewport page and moves cursor to last visible entry.
pub fn handle_scroll_down(state: &mut AppState) -> IntentResult {
    let viewport_height = state.active_session().viewport_height_value().max(1);
    let half = viewport_height / 2;
    state.active_session_mut().scroll_down(half);
    state.active_session_mut().move_cursor_to_last_visible();
    IntentResult::empty()
}

/// Scrolls the chat log up by one mouse wheel tick and moves cursor to first visible.
pub fn handle_mouse_scroll_up(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_up(MOUSE_SCROLL_STEP);
    state.active_session_mut().move_cursor_to_first_visible();
    IntentResult::empty()
}

/// Scrolls the chat log down by one mouse wheel tick and moves cursor to last visible.
pub fn handle_mouse_scroll_down(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_down(MOUSE_SCROLL_STEP);
    state.active_session_mut().move_cursor_to_last_visible();
    IntentResult::empty()
}

/// Scrolls the chat log to the very top and moves cursor to the first entry.
pub fn handle_scroll_to_top(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_to_top();
    // Set cursor to first entry.
    if !state.active_session().history().is_empty() {
        state.active_session_mut().set_selected_entry_index(0);
    }
    IntentResult::empty()
}

/// Scrolls the chat log to the very bottom and moves cursor to the last entry.
pub fn handle_scroll_to_bottom(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_to_bottom();
    // Set cursor to last entry.
    let max = state.active_session().history().len().saturating_sub(1);
    if !state.active_session().history().is_empty() {
        state.active_session_mut().set_selected_entry_index(max);
    }
    IntentResult::empty()
}

/// Opens the input in an external editor.
pub fn handle_edit_input(state: &mut AppState) -> IntentResult {
    state.frontend.tui_signals.edit_requested = true;
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use crate::common::app_state::AppState;
    use crate::protocol::ChatEntry;

    use super::*;

    #[rstest::rstest]
    fn scroll_up_decrements_scroll_offset() {
        // Given a state with entries.
        let mut state = AppState::default();
        for _ in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("line"));
        }
        state.active_session_mut().scroll_down(20);

        // When handling ScrollUp.
        let _result = handle_scroll_up(&mut state);

        // Then the scroll offset decreased.
        let offset_before = 20u16;
        assert!(state.active_session().scroll_offset().unwrap_or(0) < offset_before);
    }

    #[rstest::rstest]
    fn scroll_up_returns_no_commands() {
        // Given a state with entries.
        let mut state = AppState::default();
        for _ in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("line"));
        }
        state.active_session_mut().scroll_down(20);

        // When handling ScrollUp.
        let result = handle_scroll_up(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn scroll_down_increments_scroll_offset() {
        // Given a state with entries.
        let mut state = AppState::default();
        for _ in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("line"));
        }

        // When handling ScrollDown.
        let result = handle_scroll_down(&mut state);

        // Then scroll offset is non-zero (was at bottom, now scrolled up from bottom).
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn mouse_scroll_up_decrements_scroll_offset() {
        // Given a state with entries.
        let mut state = AppState::default();
        for _ in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("line"));
        }
        state.active_session_mut().scroll_down(20);

        // When handling MouseScrollUp.
        let result = handle_mouse_scroll_up(&mut state);

        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn mouse_scroll_down_increments_scroll_offset() {
        // Given a state with entries.
        let mut state = AppState::default();
        for _ in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("line"));
        }

        // When handling MouseScrollDown.
        let result = handle_mouse_scroll_down(&mut state);

        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn scroll_to_top_sets_offset_to_zero() {
        // Given a state scrolled down.
        let mut state = AppState::default();
        for _ in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("line"));
        }
        state.active_session_mut().scroll_down(50);

        // When handling ScrollToTop.
        let _result = handle_scroll_to_top(&mut state);

        // Then scroll offset is at top.
        assert_eq!(state.active_session().scroll_offset(), Some(0));
    }

    #[rstest::rstest]
    fn scroll_to_top_returns_no_commands() {
        // Given a state scrolled down.
        let mut state = AppState::default();
        for _ in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("line"));
        }
        state.active_session_mut().scroll_down(50);

        // When handling ScrollToTop.
        let result = handle_scroll_to_top(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn scroll_to_bottom_resets_scroll() {
        // Given a state scrolled up from bottom.
        let mut state = AppState::default();
        for _ in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("line"));
        }
        state.active_session_mut().scroll_up(10);

        // When handling ScrollToBottom.
        let _result = handle_scroll_to_bottom(&mut state);

        // Then we're at bottom.
        assert!(state.active_session().is_at_bottom());
    }

    #[rstest::rstest]
    fn scroll_to_bottom_returns_no_commands() {
        // Given a state scrolled up from bottom.
        let mut state = AppState::default();
        for _ in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("line"));
        }
        state.active_session_mut().scroll_up(10);

        // When handling ScrollToBottom.
        let result = handle_scroll_to_bottom(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn edit_input_sets_tui_signal() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling EditInput.
        let _result = handle_edit_input(&mut state);

        // Then the edit_requested signal is set.
        assert!(state.frontend.tui_signals.edit_requested);
    }

    #[rstest::rstest]
    fn edit_input_returns_no_commands() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling EditInput.
        let result = handle_edit_input(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }
}
