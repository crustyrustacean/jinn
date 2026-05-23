//! Chat entry selection intent handlers — navigate and pin entries.

use crate::common::app_state::AppState;
use crate::feat::context::protocol::command::{PinChatEntry, UnpinChatEntry};
use crate::feat::session::protocol::session_fork_requested::SessionForkRequested;
use crate::protocol::{Command, IntentResult, PinPosition};

use super::validator;
/// Selects the next chat entry in the active session.
///
/// If the cursor is on the last visible entry, pages the viewport down first,
/// then advances the cursor by exactly 1 (not jump to first visible in new viewport).
/// Clamps at the last entry in history — no wrapping.
pub fn handle_select_next(state: &mut AppState) -> IntentResult {
    validator::validate_chat_entry_select_next(state);
    let session = state.active_session_mut();
    let visible = session.visible_entry_range();
    let current = session.selected_entry_index();
    let max = session.history().len().saturating_sub(1);

    if let Some(cur) = current {
        if cur >= max {
            // Already at last entry — no-op.
            return IntentResult::empty();
        }
        // Check if cursor is at last visible entry.
        let last_visible = if visible.is_empty() {
            None
        } else {
            Some(visible.end.saturating_sub(1))
        };
        if last_visible == Some(cur) {
            // At last visible — page down, then advance by exactly 1.
            let viewport_height = session.viewport_height_value().max(1);
            session.scroll_down(viewport_height);
            session.select_next_entry();
        } else {
            session.select_next_entry();
        }
    } else if !session.history().is_empty() {
        session.select_next_entry();
    }
    IntentResult::empty()
}

/// Selects the previous chat entry in the active session.
///
/// If the cursor is on the first visible entry, pages the viewport up first,
/// then moves the cursor back by exactly 1. Clamps at entry 0 — no wrapping.
pub fn handle_select_prev(state: &mut AppState) -> IntentResult {
    validator::validate_chat_entry_select_prev(state);
    let session = state.active_session_mut();
    let visible = session.visible_entry_range();
    let current = session.selected_entry_index();

    if let Some(cur) = current {
        if cur == 0 {
            // Already at first entry — no-op.
            return IntentResult::empty();
        }
        // Check if cursor is at first visible entry.
        let first_visible = if visible.is_empty() {
            None
        } else {
            Some(visible.start)
        };
        if first_visible == Some(cur) {
            // At first visible — page up, then move back by exactly 1.
            let viewport_height = session.viewport_height_value().max(1);
            session.scroll_up(viewport_height);
            session.select_prev_entry();
        } else {
            session.select_prev_entry();
        }
    } else if !session.history().is_empty() {
        session.select_prev_entry();
    }
    IntentResult::empty()
}

/// Toggles the pin state of the currently selected chat entry.
///
/// If the entry is pinned, sends an `UnpinChatEntry` command.
/// If the entry is not pinned, sends a `PinChatEntry` command with `Relative` position.
pub fn handle_pin_selected(state: &mut AppState) -> IntentResult {
    if validator::validate_chat_entry_pin_selected(state).is_err() {
        return IntentResult::empty();
    }

    let session_id = state.session.active_session_id().clone();
    let Some(selected) = state.active_session().selected_entry() else {
        return IntentResult::empty();
    };
    let entry_id = selected.id.clone();

    if selected.is_pinned() {
        IntentResult::with_commands(vec![Command::UnpinChatEntry(UnpinChatEntry {
            session_id,
            entry_id,
        })])
    } else {
        IntentResult::with_commands(vec![Command::PinChatEntry(PinChatEntry {
            session_id,
            entry_id,
            position: PinPosition::Relative,
        })])
    }
}

/// Toggles expand/collapse of the selected tool entry (tool call or tool result).
pub fn handle_expand_tool_entry(state: &mut AppState) -> IntentResult {
    if validator::validate_expand_tool_entry(state).is_err() {
        return IntentResult::empty();
    }

    let Some(entry_id) = state.active_session().selected_entry_id().cloned() else {
        return IntentResult::empty();
    };

    state.active_session_mut().toggle_expand_entry(entry_id);
    IntentResult::empty()
}

/// Forks the session at the currently selected chat entry.
///
/// Emits a `SessionForkRequested` command with `at_ordinal` set to the
/// selected entry's index in the history. The session actor handles the
/// actual fork in SQLite and loads the new session.
///
/// # Panics
///
/// Panics if `selected_entry_index()` returns `None` after validation
/// succeeds. This should never happen — the validator guarantees a
/// selection exists.
pub fn handle_fork_from_entry(state: &mut AppState) -> IntentResult {
    if super::validator::validate_fork_from_entry(state).is_err() {
        return IntentResult::empty();
    }

    let source_session_id = state.session.active_session_id().clone();
    let at_ordinal = state
        .active_session()
        .selected_entry_index()
        .expect("validator confirmed selection exists");

    state.session.begin_load(source_session_id.clone());

    IntentResult::with_commands(vec![Command::SessionForkRequested(SessionForkRequested {
        source_session_id,
        at_ordinal,
    })])
}

/// Yanks (copies) the text of the currently selected chat entry to the clipboard.
///
/// Extracts the entry's text via [`ChatEntry::text()`] and stashes it in
/// [`TuiSignals::yank_text`] for the TUI layer to write to the system clipboard.
pub fn handle_yank_selected(state: &mut AppState) -> IntentResult {
    if validator::validate_yank_selected(state).is_err() {
        return IntentResult::empty();
    }
    let Some(entry) = state.active_session().selected_entry() else {
        return IntentResult::empty();
    };
    let text = entry.text();
    state.frontend.tui_signals.yank_text = Some(text);
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use crate::common::app_state::AppState;
    use crate::feat::context::protocol::command::PinChatEntry;
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::protocol::{ChatEntry, Command, PinPosition};

    use super::*;

    #[rstest::rstest]
    fn chat_entry_select_next_increments_index() {
        // Given a state with entries and selection at first.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        // After push, selection is at index 1 (last pushed). Move to 0.
        state.active_session_mut().select_prev_entry();
        assert_eq!(state.active_session().selected_entry_index(), Some(0));

        // When handling select next.
        let _result = handle_select_next(&mut state);

        // Then the second entry is selected.
        assert_eq!(state.active_session().selected_entry_index(), Some(1));
    }

    #[rstest::rstest]
    fn chat_entry_select_next_returns_no_commands() {
        // Given a state with entries and selection at first.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        // After push, selection is at index 1 (last pushed). Move to 0.
        state.active_session_mut().select_prev_entry();
        assert_eq!(state.active_session().selected_entry_index(), Some(0));

        // When handling select next.
        let result = handle_select_next(&mut state);

        // Then no commands are emitted.
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
        let _result = handle_select_prev(&mut state);

        // Then selection moved.
        assert_eq!(state.active_session().selected_entry_index(), Some(0));
    }

    #[rstest::rstest]
    fn chat_entry_select_prev_returns_no_commands() {
        // Given a state with entries and selection at last.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        state.active_session_mut().select_prev_entry();

        // When handling select prev.
        let result = handle_select_prev(&mut state);

        // Then no commands are emitted.
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
                Command::PinChatEntry(PinChatEntry {
                    position: PinPosition::Relative,
                    ..
                })
            )
        }));
    }

    #[rstest::rstest]
    fn chat_entry_pin_selected_noop_with_empty_history() {
        // Given a state with no history.
        let mut state = AppState::default();

        // When handling pin selected.
        let result = handle_pin_selected(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn toggle_pin_selected_returns_unpin_command_when_pinned() {
        // Given a state with a selected pinned entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();
        let entry_id = state.active_session().selected_entry_id().unwrap().clone();
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Top);

        // When handling pin selected (toggle).
        let result = handle_pin_selected(&mut state);

        // Then an UnpinChatEntry command is returned.
        assert!(result.commands.iter().any(|c| {
            matches!(
                c,
                Command::UnpinChatEntry(
                    crate::feat::context::protocol::command::UnpinChatEntry { .. }
                )
            )
        }));
    }

    #[rstest::rstest]
    fn expand_tool_entry_toggles_expanded_state() {
        // Given a state with a selected tool result.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_result(
                "id",
                "bash",
                "output",
                ToolResultStatus::Success,
            ));
        state.active_session_mut().select_next_entry();
        let entry_id = state.active_session().selected_entry_id().unwrap().clone();

        // When handling expand tool entry.
        let _result = handle_expand_tool_entry(&mut state);

        // Then the entry is expanded.
        assert!(state.active_session().is_entry_expanded(&entry_id));
    }

    #[rstest::rstest]
    fn expand_tool_entry_returns_no_commands() {
        // Given a state with a selected tool result.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_result(
                "id",
                "bash",
                "output",
                ToolResultStatus::Success,
            ));
        state.active_session_mut().select_next_entry();
        let _entry_id = state.active_session().selected_entry_id().unwrap().clone();

        // When handling expand tool entry.
        let result = handle_expand_tool_entry(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn expand_tool_entry_toggles_back_to_collapsed() {
        // Given a state with an expanded tool result.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_result(
                "id",
                "bash",
                "output",
                ToolResultStatus::Success,
            ));
        state.active_session_mut().select_next_entry();
        let entry_id = state.active_session().selected_entry_id().unwrap().clone();
        state
            .active_session_mut()
            .toggle_expand_entry(entry_id.clone());

        // When handling expand tool entry again.
        handle_expand_tool_entry(&mut state);

        // Then the entry is collapsed.
        assert!(!state.active_session().is_entry_expanded(&entry_id));
    }

    #[rstest::rstest]
    fn expand_tool_entry_noop_with_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_result(
                "id",
                "bash",
                "output",
                ToolResultStatus::Success,
            ));

        // When handling expand tool entry.
        let result = handle_expand_tool_entry(&mut state);

        // Then no change.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn expand_tool_entry_noop_with_non_tool_entry() {
        // Given a state with a selected user entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When handling expand tool entry.
        let result = handle_expand_tool_entry(&mut state);

        // Then no change.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn expand_tool_entry_toggles_tool_call_expanded_state() {
        // Given a state with a selected tool call.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::tool_call(
            "id",
            "bash",
            "{\"cmd\": true}",
        ));
        state.active_session_mut().select_next_entry();
        let entry_id = state.active_session().selected_entry_id().unwrap().clone();

        // When handling expand tool entry.
        let _result = handle_expand_tool_entry(&mut state);

        // Then the entry is expanded.
        assert!(state.active_session().is_entry_expanded(&entry_id));
    }

    #[rstest::rstest]
    fn expand_tool_entry_tool_call_returns_no_commands() {
        // Given a state with a selected tool call.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::tool_call(
            "id",
            "bash",
            "{\"cmd\": true}",
        ));
        state.active_session_mut().select_next_entry();
        let _entry_id = state.active_session().selected_entry_id().unwrap().clone();

        // When handling expand tool entry.
        let result = handle_expand_tool_entry(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn fork_from_entry_returns_fork_command() {
        // Given a state with 3 entries, middle entry selected (index 1).
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("first"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("second"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("third"));
        // Select second entry (index 1).
        state.active_session_mut().select_prev_entry();
        assert_eq!(state.active_session().selected_entry_index(), Some(1));

        // When handling fork from entry.
        let result = handle_fork_from_entry(&mut state);

        // Then a SessionForkRequested command is returned with at_ordinal == 1.
        assert!(result.commands.iter().any(|c| {
            matches!(
                c,
                Command::SessionForkRequested(
                    crate::feat::session::protocol::session_fork_requested::SessionForkRequested {
                        at_ordinal: 1,
                        ..
                    }
                )
            )
        }));
    }

    #[rstest::rstest]
    fn fork_from_entry_noop_with_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().clear_selection();

        // When handling fork from entry.
        let result = handle_fork_from_entry(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn fork_from_entry_noop_with_empty_history() {
        // Given a state with no history.
        let mut state = AppState::default();

        // When handling fork from entry.
        let result = handle_fork_from_entry(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn fork_from_entry_uses_selected_index_as_ordinal() {
        // Given a state with 5 entries, entry at index 3 selected.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("d"));
        state.active_session_mut().push_entry(ChatEntry::user("e"));
        // Select entry at index 3 ("d").
        state.active_session_mut().select_prev_entry();
        assert_eq!(state.active_session().selected_entry_index(), Some(3));

        // When handling fork from entry.
        let result = handle_fork_from_entry(&mut state);

        // Then at_ordinal is 3.
        let ordinal = result.commands.iter().find_map(|c| match c {
            Command::SessionForkRequested(req) => Some(req.at_ordinal),
            _ => None,
        });
        assert_eq!(ordinal, Some(3));
    }
}
