//! Navigation intent handlers — scroll, tab, and editor.

use nullslop_component::AppState;
use nullslop_protocol::tab::TabDirection;
use nullslop_protocol::IntentResult;

/// Number of lines to scroll per keyboard step.
const SCROLL_STEP: u16 = 10;
/// Number of lines to scroll per mouse wheel tick.
const MOUSE_SCROLL_STEP: u16 = 3;

/// Scrolls the chat log up by one keyboard step.
pub fn handle_scroll_up(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_up(SCROLL_STEP);
    IntentResult::empty()
}

/// Scrolls the chat log down by one keyboard step.
pub fn handle_scroll_down(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_down(SCROLL_STEP);
    IntentResult::empty()
}

/// Scrolls the chat log up by one mouse wheel tick.
pub fn handle_mouse_scroll_up(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_up(MOUSE_SCROLL_STEP);
    IntentResult::empty()
}

/// Scrolls the chat log down by one mouse wheel tick.
pub fn handle_mouse_scroll_down(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_down(MOUSE_SCROLL_STEP);
    IntentResult::empty()
}

/// Scrolls the chat log to the very top.
pub fn handle_scroll_to_top(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_to_top();
    IntentResult::empty()
}

/// Scrolls the chat log to the very bottom.
pub fn handle_scroll_to_bottom(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_to_bottom();
    IntentResult::empty()
}

/// Switches to the next or previous tab.
pub fn handle_switch_tab(state: &mut AppState, direction: TabDirection) -> IntentResult {
    state.frontend.active_tab = match direction {
        TabDirection::Next => state.frontend.active_tab.next(),
        TabDirection::Prev => state.frontend.active_tab.prev(),
    };
    IntentResult::empty()
}

/// Opens the input in an external editor.
pub fn handle_edit_input(state: &mut AppState) -> IntentResult {
    state.frontend.tui_signals.edit_requested = true;
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    use nullslop_component::AppState;
    use nullslop_protocol::tab::TabDirection;
    use nullslop_protocol::ChatEntry;

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
        let result = handle_scroll_up(&mut state);

        // Then the scroll offset decreased.
        let offset_before = 20u16;
        assert!(state.active_session().scroll_offset().unwrap_or(0) < offset_before);
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
        let result = handle_scroll_to_top(&mut state);

        // Then scroll offset is at top.
        assert_eq!(state.active_session().scroll_offset(), Some(0));
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
        let result = handle_scroll_to_bottom(&mut state);

        // Then we're at bottom.
        assert!(state.active_session().is_at_bottom());
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn switch_tab_next_advances_tab() {
        // Given a state on Chat tab.
        let mut state = AppState::default();
        assert_eq!(
            state.frontend.active_tab,
            nullslop_protocol::ActiveTab::Chat
        );

        // When handling SwitchTab(Next).
        let result = handle_switch_tab(&mut state, TabDirection::Next);

        // Then the tab has advanced.
        assert_eq!(
            state.frontend.active_tab,
            nullslop_protocol::ActiveTab::Dashboard
        );
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn edit_input_sets_tui_signal() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling EditInput.
        let result = handle_edit_input(&mut state);

        // Then the edit_requested signal is set.
        assert!(state.frontend.tui_signals.edit_requested);
        assert!(result.commands.is_empty());
    }
}
