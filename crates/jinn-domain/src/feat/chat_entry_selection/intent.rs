//! Chat entry selection intent handlers - navigate and pin entries.

use crate::common::app_state::AppState;
use crate::feat::context::protocol::command::{PinChatEntry, UnpinChatEntry};
use crate::feat::session::protocol::session_fork_requested::SessionForkRequested;
use crate::feat::session::ChatSessionState;
use crate::feat::ui::chat_log::visual_item::VisualItem;
use crate::protocol::{Command, ContextOverride, IntentResult, PinPosition};

use super::validator;

/// Advances the selection cursor by one entry, paging the viewport if needed.
///
/// Returns `false` if already at the last entry (no-op).
pub(crate) fn advance_selection_one(session: &mut ChatSessionState) -> bool {
    let visible = session.visible_entry_range();
    let current = session.selected_entry_index();
    let max = if session.visual_items().is_empty() {
        session.history().len().saturating_sub(1)
    } else {
        session.visual_items().len().saturating_sub(1)
    };

    let Some(cur) = current else {
        // No selection - select first entry if history exists.
        if !session.history().is_empty() {
            session.select_next_entry();
            return true;
        }
        return false;
    };

    if cur >= max {
        return false; // At bottom - no-op.
    }

    let last_visible = if visible.is_empty() {
        None
    } else {
        Some(visible.end.saturating_sub(1))
    };

    if last_visible == Some(cur) {
        let viewport_height = session.viewport_height_value().max(1);
        session.scroll_down(viewport_height);
    }
    session.select_next_entry();
    true
}
/// Selects the next chat entry in the active session.
///
/// If the cursor is on the last visible entry, pages the viewport down first,
/// then advances the cursor by exactly 1 (not jump to first visible in new viewport).
/// Clamps at the last entry in history - no wrapping.
pub fn handle_select_next(state: &mut AppState) -> IntentResult {
    validator::validate_chat_entry_select_next(state);
    advance_selection_one(state.active_session_mut());
    IntentResult::empty()
}

/// Selects the previous chat entry in the active session.
///
/// If the cursor is on the first visible entry, pages the viewport up first,
/// then moves the cursor back by exactly 1. Clamps at entry 0 - no wrapping.
pub fn handle_select_prev(state: &mut AppState) -> IntentResult {
    validator::validate_chat_entry_select_prev(state);
    let session = state.active_session_mut();
    let visible = session.visible_entry_range();
    let current = session.selected_entry_index();

    if let Some(cur) = current {
        if cur == 0 {
            // Already at first entry - no-op.
            return IntentResult::empty();
        }
        // Check if cursor is at first visible entry.
        let first_visible = if visible.is_empty() {
            None
        } else {
            Some(visible.start)
        };
        if first_visible == Some(cur) {
            // At first visible - page up, then move back by exactly 1.
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

/// Toggles visibility of ignored entries in the selected visual item's block.
///
/// - If the selected item is a `CollapsedIgnoredBlock` → expand it.
/// - If the selected item is an ignored entry in an expanded block → collapse the block.
/// - If the selected item is a non-ignored entry → no-op.
/// - If nothing is selected → no-op.
pub fn handle_toggle_ignored_block(state: &mut AppState) -> IntentResult {
    let session = state.active_session();
    let Some(vi_idx) = session.selected_entry_index() else {
        return IntentResult::empty();
    };

    let history = session.history();
    let items = session.visual_items();

    let entry_id = match items.get(vi_idx) {
        Some(VisualItem::CollapsedIgnoredBlock { start, .. }) => {
            // Block is collapsed → expand it.
            history[*start].id.clone()
        }
        Some(VisualItem::Entry(hist_idx)) => {
            // Entry might be in an expanded ignored block → collapse it.
            let entry = &history[*hist_idx];
            if entry.is_in_context() {
                return IntentResult::empty();
            }
            entry.id.clone()
        }
        None => return IntentResult::empty(),
    };

    drop(items);
    state
        .active_session_mut()
        .toggle_ignored_block_visibility(&entry_id);
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
/// succeeds. This should never happen - the validator guarantees a
/// selection exists.
pub fn handle_fork_from_entry(state: &mut AppState) -> IntentResult {
    if super::validator::validate_fork_from_entry(state).is_err() {
        return IntentResult::empty();
    }

    let source_session_id = state.session.active_session_id().clone();
    let at_ordinal = state
        .active_session()
        .selected_history_index()
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

/// Toggle the `ignored` flag on the currently selected chat entry, with
/// sweep support for holding `x`.
///
/// **First press:** validates, toggles the entry, captures the resulting
/// `ContextOverride`, advances the cursor, and stores the sweep state.
///
/// **Subsequent presses within 100ms:** applies the captured state (not a
/// toggle) to the now-selected entry, advances the cursor, and refreshes
/// the sweep timestamp. Pinned entries are skipped during the sweep.
///
/// The sweep state is cleared by either a >100ms gap or any non-
/// `ChatEntryIgnoreSelected` intent.
///
/// # Panics
///
/// Calls `expect` on selected entry after validation; should never panic in practice.
pub fn handle_ignore_selected(state: &mut AppState) -> IntentResult {
    // Try to continue an existing sweep.
    if let Some(target) = state.active_session_mut().take_ignore_sweep() {
        // Sweep continuation - apply fixed state, skip pinned entries.
        loop {
            let session = state.active_session_mut();

            // If no entry is selected or we can't advance, stop.
            if session.selected_entry_index().is_none() {
                return IntentResult::empty();
            }

            // Check if the currently selected entry is pinned.
            let is_pinned = session
                .selected_entry()
                .is_some_and(crate::feat::session::chat_entry::ChatEntry::is_pinned);

            if is_pinned {
                // Skip pinned - try advancing to the next entry.
                if !advance_selection_one(session) {
                    // At bottom, pinned is the last entry - stop.
                    return IntentResult::empty();
                }
                // Loop to check the new entry.
                continue;
            }

            // Apply the captured state directly (not a toggle).
            session.set_entry_context_override(target);

            // Advance cursor for next press.
            advance_selection_one(session);

            // Re-store sweep state with fresh timestamp.
            session.set_ignore_sweep(target);

            let session_id = state.active_session().session_id().clone();
            return IntentResult::with_commands(vec![Command::PersistSession(
                crate::feat::session_lifecycle::protocol::command::PersistSession { session_id },
            )]);
        }
    }

    // Fresh press (no active sweep) - validate and toggle.
    if validator::validate_chat_entry_ignore_selected(state).is_err() {
        return IntentResult::empty();
    }

    // Toggle the entry's ignore state.
    state.active_session_mut().toggle_entry_ignored();

    // Capture the resulting state on the toggled entry.
    let captured = state
        .active_session()
        .selected_entry()
        .expect("validator confirmed selection exists")
        .context_override;

    // Advance cursor.
    advance_selection_one(state.active_session_mut());

    // Store sweep state for potential continuation - only when the result
    // is a forced override. A Default result means the user reverted an entry
    // to its kind default; that's not a sweep action.
    if captured != ContextOverride::Default {
        state.active_session_mut().set_ignore_sweep(captured);
    }

    let session_id = state.active_session().session_id().clone();
    IntentResult::with_commands(vec![Command::PersistSession(
        crate::feat::session_lifecycle::protocol::command::PersistSession { session_id },
    )])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use crate::common::app_state::AppState;
    use crate::feat::context::protocol::command::PinChatEntry;
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::protocol::{ChatEntry, Command, ContextOverride, PinPosition};

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

    // --- Yank Selected Entry ---

    #[rstest::rstest]
    fn yank_selected_sets_yank_text_when_entry_selected() {
        // Given a state with a selected user entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When handling yank selected.
        let _result = handle_yank_selected(&mut state);

        // Then yank_text is set to the entry text.
        assert_eq!(
            state.frontend.tui_signals.yank_text,
            Some("hello".to_string())
        );
    }

    #[rstest::rstest]
    fn yank_selected_returns_no_commands() {
        // Given a state with a selected entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When handling yank selected.
        let result = handle_yank_selected(&mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn yank_selected_noop_with_no_selection() {
        // Given a state with no entries.
        let mut state = AppState::default();

        // When handling yank selected.
        let result = handle_yank_selected(&mut state);

        // Then yank_text is not set and no commands are emitted.
        assert!(state.frontend.tui_signals.yank_text.is_none());
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn yank_selected_preserves_selection_index() {
        // Given a state with 2 entries, first selected.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));
        state.active_session_mut().select_prev_entry();
        assert_eq!(state.active_session().selected_entry_index(), Some(0));

        // When handling yank selected.
        let _result = handle_yank_selected(&mut state);

        // Then the selection index is unchanged.
        assert_eq!(state.active_session().selected_entry_index(), Some(0));
    }

    #[rstest::rstest]
    fn yank_selected_extracts_assistant_text() {
        // Given a state with a selected assistant entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("response text"));
        state.active_session_mut().select_next_entry();

        // When handling yank selected.
        let _result = handle_yank_selected(&mut state);

        // Then yank_text contains the assistant text.
        assert_eq!(
            state.frontend.tui_signals.yank_text,
            Some("response text".to_string())
        );
    }

    #[rstest::rstest]
    fn yank_selected_extracts_tool_result_text() {
        // Given a state with a selected tool result.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_result(
                "id",
                "bash",
                "output text",
                ToolResultStatus::Success,
            ));
        state.active_session_mut().select_next_entry();

        // When handling yank selected.
        let _result = handle_yank_selected(&mut state);

        // Then yank_text contains "bash: output text" (ChatEntry.text() format).
        assert_eq!(
            state.frontend.tui_signals.yank_text,
            Some("bash: output text".to_string())
        );
    }

    // --- Toggle Ignored Block ---

    #[rstest::rstest]
    fn toggle_ignored_block_noop_with_no_selection() {
        // Given a state with no selection.
        let mut state = AppState::default();

        // When handling toggle ignored block.
        let result = handle_toggle_ignored_block(&mut state);

        // Then no commands are emitted and no state change.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn toggle_ignored_block_noop_with_non_ignored_entry() {
        // Given a state with a selected non-ignored entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When handling toggle ignored block.
        let result = handle_toggle_ignored_block(&mut state);

        // Then no commands emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn toggle_ignored_block_expands_collapsed_block() {
        // Given a session with a collapsed ignored block selected.
        use crate::feat::ui::chat_log::visual_item::{
            DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, VisualItem, build_visual_items,
        };

        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        for _ in 0..15 {
            let mut entry = ChatEntry::user("ignored");
            entry.context_override = crate::protocol::ContextOverride::ForcedExclude;
            state.active_session_mut().push_entry(entry);
        }
        state.active_session_mut().push_entry(ChatEntry::user("b"));

        let items = build_visual_items(
            state.active_session().history(),
            &state.active_session().ui.shown_ignored_blocks,
            PROXIMITY_COUNT,
            DEFAULT_MIN_COLLAPSE_COUNT,
        );
        state.active_session_mut().set_visual_items(items.clone());

        // Find the collapsed block's visual-item index.
        let collapsed_vi_idx = items
            .iter()
            .position(|i| matches!(i, VisualItem::CollapsedIgnoredBlock { .. }))
            .expect("should have a collapsed block");
        state
            .active_session_mut()
            .set_selected_entry_index(collapsed_vi_idx);

        let block_start_id = state.active_session().history()[1].id.clone();
        assert!(
            !state
                .active_session()
                .ui
                .shown_ignored_blocks
                .contains(&block_start_id),
            "block should start collapsed"
        );

        // When handling toggle ignored block.
        let _result = handle_toggle_ignored_block(&mut state);

        // Then the block is expanded.
        assert!(
            state
                .active_session()
                .ui
                .shown_ignored_blocks
                .contains(&block_start_id),
            "block should be shown after toggle on collapsed block"
        );
    }

    #[rstest::rstest]
    fn toggle_ignored_block_collapses_expanded_block() {
        // Given a session with an expanded ignored block, an ignored entry selected.
        use crate::feat::ui::chat_log::visual_item::{
            DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, build_visual_items,
        };

        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        for _ in 0..15 {
            let mut entry = ChatEntry::user("ignored");
            entry.context_override = crate::protocol::ContextOverride::ForcedExclude;
            state.active_session_mut().push_entry(entry);
        }
        state.active_session_mut().push_entry(ChatEntry::user("b"));

        let block_start_id = state.active_session().history()[1].id.clone();

        // Expand the block first.
        let entry_5_id = state.active_session().history()[5].id.clone();
        state
            .active_session_mut()
            .toggle_ignored_block_visibility(&entry_5_id);
        assert!(
            state
                .active_session()
                .ui
                .shown_ignored_blocks
                .contains(&block_start_id),
            "block should be expanded"
        );

        // Rebuild visual items (now expanded - individual Entry items).
        let items = build_visual_items(
            state.active_session().history(),
            &state.active_session().ui.shown_ignored_blocks,
            PROXIMITY_COUNT,
            DEFAULT_MIN_COLLAPSE_COUNT,
        );
        state.active_session_mut().set_visual_items(items.clone());

        // Select an ignored entry within the expanded block (history index 5).
        let target_vi_idx = items
            .iter()
            .position(|i| {
                matches!(i, crate::feat::ui::chat_log::visual_item::VisualItem::Entry(hist_idx) if *hist_idx == 5)
            })
            .expect("should find ignored entry at history index 5");
        state
            .active_session_mut()
            .set_selected_entry_index(target_vi_idx);

        // When handling toggle ignored block.
        let _result = handle_toggle_ignored_block(&mut state);

        // Then the block is collapsed.
        assert!(
            !state
                .active_session()
                .ui
                .shown_ignored_blocks
                .contains(&block_start_id),
            "block should be collapsed after toggle on ignored entry"
        );
    }

    // --- Ignore selected tests ---

    #[rstest::rstest]
    fn handle_ignore_selected_toggles_false_to_true() {
        // Given a state with a selected user entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then the entry is now ignored.
        let selected = state.active_session().selected_entry().expect("entry");
        assert!(selected.ignored(), "entry should be ignored after toggle");
        // And a PersistSession command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| { matches!(c, Command::PersistSession(_)) }),
            "should contain PersistSession command"
        );
    }

    #[rstest::rstest]
    fn handle_ignore_selected_toggles_true_to_false() {
        // Given a state with a selected ignored user entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello").with_ignored(true));
        state.active_session_mut().select_next_entry();

        // When handling ignore selected.
        let _result = handle_ignore_selected(&mut state);

        // Then the entry is now un-ignored.
        let selected = state.active_session().selected_entry().expect("entry");
        assert!(
            !selected.ignored(),
            "entry should be un-ignored after toggle"
        );
    }

    #[rstest::rstest]
    fn handle_ignore_selected_noop_empty_history() {
        // Given an empty session.
        let mut state = AppState::default();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then no commands are emitted.
        assert!(
            result.commands.is_empty(),
            "empty history should produce no commands"
        );
    }

    #[rstest::rstest]
    fn handle_ignore_selected_noop_no_selection() {
        // Given a session with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().clear_selection();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then no commands are emitted.
        assert!(
            result.commands.is_empty(),
            "no selection should produce no commands"
        );
    }

    #[rstest::rstest]
    fn handle_ignore_selected_noop_pinned_entry() {
        // Given a state with a selected pinned entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello").with_pin(PinPosition::Top));
        state.active_session_mut().select_next_entry();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then no commands are emitted and ignored is unchanged.
        assert!(
            result.commands.is_empty(),
            "pinned entry should produce no commands"
        );
        let selected = state.active_session().selected_entry().expect("entry");
        assert!(
            !selected.ignored(),
            "pinned entry ignored should stay false"
        );
    }

    #[rstest::rstest]
    fn handle_ignore_selected_toggles_system_entry() {
        // Given a state with a selected system entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::system("system prompt"));
        state.active_session_mut().select_next_entry();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then the entry is toggled (System is excluded by default, so toggle → ForcedInclude).
        assert!(
            !result.commands.is_empty(),
            "system entry toggle should produce commands"
        );
        let selected = state.active_session().selected_entry().expect("entry");
        assert_eq!(selected.context_override, ContextOverride::ForcedInclude);
    }

    #[rstest::rstest]
    fn handle_ignore_selected_toggles_thinking_entry() {
        // Given a state with a selected thinking entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::thinking("thinking..."));
        state.active_session_mut().select_next_entry();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then the entry is toggled (Thinking is excluded by default, so toggle → ForcedInclude).
        assert!(
            !result.commands.is_empty(),
            "thinking entry toggle should produce commands"
        );
        let selected = state.active_session().selected_entry().expect("entry");
        assert_eq!(selected.context_override, ContextOverride::ForcedInclude);
    }

    #[rstest::rstest]
    fn handle_ignore_selected_toggles_transient_entry() {
        // Given a state with a selected transient entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::transient("ephemeral"));
        state.active_session_mut().select_next_entry();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then the entry is toggled (Transient is excluded by default, so toggle → ForcedInclude).
        assert!(
            !result.commands.is_empty(),
            "transient entry toggle should produce commands"
        );
        let selected = state.active_session().selected_entry().expect("entry");
        assert_eq!(selected.context_override, ContextOverride::ForcedInclude);
    }

    #[rstest::rstest]
    fn handle_ignore_selected_toggles_compaction_entry() {
        // Given a state with a selected compaction entry.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry {
            id: crate::protocol::ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: crate::protocol::ChatEntryKind::Compaction {
                summary: "summary".to_owned(),
                tokens_before: 100,
                tokens_after: 50,
                entries_compacted: 5,
                model_used: "test/model".to_owned(),
            },
            pin_position: None,
            context_override: crate::protocol::ContextOverride::Default,
        });
        state.active_session_mut().select_next_entry();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then the entry is toggled (Compaction is included by default, so toggle → ForcedExclude).
        assert!(
            !result.commands.is_empty(),
            "compaction entry toggle should produce commands"
        );
        let selected = state.active_session().selected_entry().expect("entry");
        assert_eq!(selected.context_override, ContextOverride::ForcedExclude);
    }

    // --- Sweep tests ---

    #[rstest::rstest]
    fn sweep_first_press_toggles_and_captures_state() {
        // Given a session with 3 user entries, first selected.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));
        // Select first entry.
        state.active_session_mut().select_prev_entry();
        state.active_session_mut().select_prev_entry();
        assert_eq!(state.active_session().selected_entry_index(), Some(0));

        // When handling ignore selected (first press).
        let result = handle_ignore_selected(&mut state);

        // Then entry 0 is toggled to ForcedExclude.
        assert_eq!(
            state.active_session().history()[0].context_override,
            ContextOverride::ForcedExclude
        );
        // And the cursor has advanced to entry 1.
        assert_eq!(state.active_session().selected_entry_index(), Some(1));
        // And sweep state is stored with ForcedExclude target.
        let sweep = state.active_session_mut().take_ignore_sweep();
        assert_eq!(sweep, Some(ContextOverride::ForcedExclude));
        // And a PersistSession command is returned.
        assert!(result.commands.iter().any(|c| matches!(c, Command::PersistSession(_))));
    }

    #[rstest::rstest]
    fn sweep_second_press_applies_captured_state() {
        // Given a session with 3 user entries, entry 0 already toggled to
        // ForcedExclude, cursor at entry 1, sweep active with ForcedExclude.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));
        state.active_session_mut().select_prev_entry();
        state.active_session_mut().select_prev_entry();

        // First press - toggles entry 0, advances to 1.
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(state.active_session().selected_entry_index(), Some(1));

        // When handling ignore selected again (second press, within 100ms).
        let _result = handle_ignore_selected(&mut state);

        // Then entry 1 is now ForcedExclude (applied, not toggled).
        assert_eq!(
            state.active_session().history()[1].context_override,
            ContextOverride::ForcedExclude
        );
        // And cursor has advanced to entry 2.
        assert_eq!(state.active_session().selected_entry_index(), Some(2));
        // And sweep state is still active.
        assert!(state.active_session_mut().take_ignore_sweep().is_some());
    }

    #[rstest::rstest]
    fn sweep_third_press_continues_to_bottom() {
        // Given a session with 3 user entries, sweep already applied to 0 and 1,
        // cursor at entry 2.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));
        state.active_session_mut().select_prev_entry();
        state.active_session_mut().select_prev_entry();

        // First press.
        let _result = handle_ignore_selected(&mut state);
        // Second press.
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(state.active_session().selected_entry_index(), Some(2));

        // When handling ignore selected (third press).
        let result = handle_ignore_selected(&mut state);

        // Then entry 2 is ForcedExclude.
        assert_eq!(
            state.active_session().history()[2].context_override,
            ContextOverride::ForcedExclude
        );
        // And cursor stays at entry 2 (at bottom, can't advance further).
        assert_eq!(state.active_session().selected_entry_index(), Some(2));
        // And commands are still returned.
        assert!(result.commands.iter().any(|c| matches!(c, Command::PersistSession(_))));
    }

    #[rstest::rstest]
    fn sweep_stops_at_bottom() {
        // Given a single entry session.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("only"));
        state.active_session_mut().select_next_entry();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then the entry is toggled.
        assert_eq!(
            state.active_session().history()[0].context_override,
            ContextOverride::ForcedExclude
        );
        // And cursor stays at 0 (at bottom).
        assert_eq!(state.active_session().selected_entry_index(), Some(0));
        // And sweep state is stored (ready if more entries appear).
        assert!(state.active_session_mut().take_ignore_sweep().is_some());
        // And commands are returned.
        assert!(!result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sweep_expired_resets_to_toggle() {
        // Given a session with 2 user entries, sweep was started but expired.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        state.active_session_mut().select_prev_entry();

        // First press - toggles entry 0, advances to 1.
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(state.active_session().selected_entry_index(), Some(1));

        // Expire the sweep by setting a stale timestamp.
        state.active_session_mut().ui.ignore_sweep = Some((
            std::time::Instant::now().checked_sub(std::time::Duration::from_millis(200)).unwrap(),
            ContextOverride::ForcedExclude,
        ));

        // When handling ignore selected again (after timeout).
        let _result = handle_ignore_selected(&mut state);

        // Then entry 1 is toggled (fresh toggle: Default → ForcedExclude).
        assert_eq!(
            state.active_session().history()[1].context_override,
            ContextOverride::ForcedExclude
        );
        // And sweep state is refreshed with a new timestamp.
        let sweep = state.active_session_mut().ui.ignore_sweep.take();
        assert!(sweep.is_some(), "sweep state should be re-stored");
        let (instant, target) = sweep.unwrap();
        assert!(instant.elapsed() < std::time::Duration::from_millis(10));
        assert_eq!(target, ContextOverride::ForcedExclude);
    }

    #[rstest::rstest]
    fn sweep_skips_pinned_entries() {
        // Given entries [user, user_pinned, user], entry 0 selected.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        let pinned_entry = ChatEntry::user("pinned").with_pin(PinPosition::Top);
        state.active_session_mut().push_entry(pinned_entry);
        state.active_session_mut().push_entry(ChatEntry::user("c"));
        state.active_session_mut().select_prev_entry();
        state.active_session_mut().select_prev_entry();
        assert_eq!(state.active_session().selected_entry_index(), Some(0));

        // First press - toggles entry 0, advances to entry 1 (pinned).
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(state.active_session().selected_entry_index(), Some(1));
        assert_eq!(
            state.active_session().history()[0].context_override,
            ContextOverride::ForcedExclude
        );

        // When handling second press (cursor on pinned entry 1).
        let _result = handle_ignore_selected(&mut state);

        // Then pinned entry 1 is unchanged (still Default).
        assert_eq!(
            state.active_session().history()[1].context_override,
            ContextOverride::Default
        );
        // And entry 2 (after pinned) gets ForcedExclude.
        assert_eq!(
            state.active_session().history()[2].context_override,
            ContextOverride::ForcedExclude
        );
        // And cursor has advanced past the pinned entry to entry 2 (or beyond).
        assert_eq!(state.active_session().selected_entry_index(), Some(2));
    }

    #[rstest::rstest]
    fn sweep_skips_pinned_at_bottom_stops() {
        // Given entries [user, pinned], entry 0 selected.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(
            ChatEntry::user("pinned").with_pin(PinPosition::Top),
        );
        state.active_session_mut().select_prev_entry();

        // First press - toggles entry 0, advances to entry 1 (pinned).
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(state.active_session().selected_entry_index(), Some(1));

        // When handling second press (cursor on pinned entry 1, at bottom).
        let result = handle_ignore_selected(&mut state);

        // Then pinned entry is unchanged.
        assert_eq!(
            state.active_session().history()[1].context_override,
            ContextOverride::Default
        );
        // And no commands returned (no-op: pinned at bottom).
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sweep_not_started_on_toggle_to_default() {
        // Given a session with 2 user entries, entry 0 already ForcedExclude.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("a").with_ignored(true));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        state.active_session_mut().select_prev_entry();
        assert_eq!(state.active_session().selected_entry_index(), Some(0));

        // When handling ignore selected (toggle from ForcedExclude → Default).
        let _result = handle_ignore_selected(&mut state);

        // Then entry 0 is now Default (un-ignored).
        assert_eq!(
            state.active_session().history()[0].context_override,
            ContextOverride::Default
        );
        // And cursor has advanced.
        assert_eq!(state.active_session().selected_entry_index(), Some(1));
        // And NO sweep state is stored (Default is not a sweep target).
        assert!(
            state.active_session_mut().take_ignore_sweep().is_none(),
            "toggling to Default should not start a sweep"
        );
    }
}
