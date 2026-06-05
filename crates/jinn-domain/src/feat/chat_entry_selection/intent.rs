//! Chat entry selection intent handlers - navigate and pin entries.

use crate::common::app_state::AppState;
use crate::feat::context::protocol::command::{PinChatEntry, UnpinChatEntry};
use crate::feat::session::ChatSessionState;
use crate::feat::session::protocol::session_fork_requested::SessionForkRequested;
use crate::feat::ui::chat_log::visual_item::VisualItem;
use crate::protocol::{Command, ContextOverride, Event, IntentResult, PinPosition};

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
/// No longer panics — returns `IntentResult::empty()` if the selected
/// entry cannot be resolved after validation.
pub fn handle_fork_from_entry(state: &mut AppState) -> IntentResult {
    if super::validator::validate_fork_from_entry(state).is_err() {
        return IntentResult::empty();
    }

    let source_session_id = state.session.active_session_id().clone();
    let Some(at_ordinal) = state.active_session().selected_history_index() else {
        return IntentResult::empty();
    };

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
/// Returns gracefully if the selected entry cannot be resolved after
/// validation (e.g. collapsed ignored block).
// FIXME: fix this absolute shitshow
pub fn handle_ignore_selected(state: &mut AppState) -> IntentResult {
    // Try to continue an existing sweep.
    if let Some(target) = state.active_session_mut().take_ignore_sweep() {
        // Sweep continuation - apply fixed state, skip pinned/collapsed.
        let mut changed_ids: Vec<crate::protocol::ChatEntryId> = Vec::new();
        loop {
            let session = state.active_session_mut();

            // If no entry is selected or we can't advance, stop.
            if session.selected_entry_index().is_none() {
                return IntentResult::empty();
            }

            // Skip pinned entries and collapsed blocks.
            let is_pinned = session
                .selected_entry()
                .is_some_and(crate::feat::session::chat_entry::ChatEntry::is_pinned);
            if is_pinned || session.is_selected_collapsed_block() {
                if !advance_selection_one(session) {
                    return IntentResult::empty();
                }
                continue;
            }

            // Apply the captured state directly (not a toggle).
            if let Some(id) = session.set_entry_context_override(target) {
                changed_ids.push(id);
            }
            session.rebuild_visual_items();

            // Rebuild may have moved cursor onto a collapsed block.
            // If so, advance and loop again.
            if session.selected_entry().is_none() {
                if !advance_selection_one(session) {
                    let session_id = session.session_id().clone();
                    session.set_ignore_sweep(target);
                    return finalize_sweep(state, session_id, changed_ids);
                }
                continue;
            }

            let Some(selected) = session.selected_entry() else {
                let session_id = session.session_id().clone();
                session.set_ignore_sweep(target);
                return finalize_sweep(state, session_id, changed_ids);
            };
            let entry_id = selected.id.clone();

            // Propagate shown state to new sub-blocks when un-ignoring.
            if matches!(
                target,
                ContextOverride::Default | ContextOverride::ForcedInclude
            ) {
                session.propagate_shown_on_unignore(&entry_id);
            }

            // Advance cursor for next press.
            advance_selection_one(session);
            session.set_ignore_sweep(target);

            let session_id = state.active_session().session_id().clone();
            return finalize_sweep(state, session_id, changed_ids);
        }
    }

    // Fresh press (no active sweep) - validate and toggle.

    // If cursor is on a collapsed block, skip past it before validation.
    // Validation calls selected_entry() which returns None for collapsed blocks.
    if state.active_session().is_selected_collapsed_block() {
        advance_selection_one(state.active_session_mut());
        return IntentResult::empty();
    }

    if validator::validate_chat_entry_ignore_selected(state).is_err() {
        return IntentResult::empty();
    }

    // Toggle the entry's ignore state.
    let maybe_entry_id = state.active_session_mut().toggle_entry_ignored();

    // Capture the resulting state on the toggled entry.
    let Some(selected) = state.active_session().selected_entry() else {
        return IntentResult::empty();
    };
    let captured = selected.context_override();
    let entry_id = selected.id.clone();

    // Propagate shown blocks if this toggle brought an entry into context
    // and split a previously shown excluded block.
    state
        .active_session_mut()
        .propagate_shown_on_unignore(&entry_id);

    // Advance cursor.
    advance_selection_one(state.active_session_mut());

    // Store sweep state for potential continuation.
    state.active_session_mut().set_ignore_sweep(captured);

    let session_id = state.active_session().session_id().clone();

    IntentResult::with_commands_and_events(
        vec![Command::PersistSession(
            crate::feat::session_lifecycle::protocol::command::PersistSession {
                session_id: session_id.clone(),
            },
        )],
        maybe_entry_id
            .map(|id| {
                vec![Event::ContextOverrideChanged(
                    crate::feat::context::protocol::event::ContextOverrideChanged {
                        session_id,
                        entry_id: id,
                    },
                )]
            })
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use crate::common::app_state::AppState;
    use crate::feat::context::protocol::command::PinChatEntry;
    use crate::feat::session::chat_entry::ChangeSource;
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
            entry.apply_context_override(
                crate::protocol::ContextOverride::ForcedExclude,
                ChangeSource::Internal {
                    label: "test".into(),
                },
            );
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
            entry.apply_context_override(
                crate::protocol::ContextOverride::ForcedExclude,
                ChangeSource::Internal {
                    label: "test".into(),
                },
            );
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
    fn handle_ignore_selected_emits_context_override_changed() {
        // Given a state with a selected user entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();
        let session_id = state.active_session().session_id().clone();
        let entry_id = state
            .active_session()
            .selected_entry()
            .expect("entry")
            .id
            .clone();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then ContextOverrideChanged event is emitted.
        let has_event = result.events.iter().any(|e| {
            matches!(
                e,
                Event::ContextOverrideChanged(payload)
                if payload.session_id == session_id && payload.entry_id == entry_id
            )
        });
        assert!(has_event, "should emit ContextOverrideChanged event");
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

        // Then no commands or events are emitted.
        assert!(
            result.commands.is_empty(),
            "empty history should produce no commands"
        );
        assert!(
            result.events.is_empty(),
            "empty history should produce no events"
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
        assert_eq!(selected.context_override(), ContextOverride::ForcedInclude);
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
        assert_eq!(selected.context_override(), ContextOverride::ForcedInclude);
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
        assert_eq!(selected.context_override(), ContextOverride::ForcedInclude);
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
            context_history: Vec::new(),
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
        assert_eq!(selected.context_override(), ContextOverride::ForcedExclude);
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
            state.active_session().history()[0].context_override(),
            ContextOverride::ForcedExclude
        );
        // And the cursor has advanced to entry 1.
        assert_eq!(state.active_session().selected_entry_index(), Some(1));
        // And sweep state is stored with ForcedExclude target.
        let sweep = state.active_session_mut().take_ignore_sweep();
        assert_eq!(sweep, Some(ContextOverride::ForcedExclude));
        // And a PersistSession command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::PersistSession(_)))
        );
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
            state.active_session().history()[1].context_override(),
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
            state.active_session().history()[2].context_override(),
            ContextOverride::ForcedExclude
        );
        // And cursor stays at entry 2 (at bottom, can't advance further).
        assert_eq!(state.active_session().selected_entry_index(), Some(2));
        // And commands are still returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::PersistSession(_)))
        );
    }

    #[rstest::rstest]
    fn sweep_stops_at_bottom() {
        // Given a single entry session.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("only"));
        state.active_session_mut().select_next_entry();

        // When handling ignore selected.
        let result = handle_ignore_selected(&mut state);

        // Then the entry is toggled.
        assert_eq!(
            state.active_session().history()[0].context_override(),
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
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(200))
                .unwrap(),
            ContextOverride::ForcedExclude,
        ));

        // When handling ignore selected again (after timeout).
        let _result = handle_ignore_selected(&mut state);

        // Then entry 1 is toggled (fresh toggle: Default → ForcedExclude).
        assert_eq!(
            state.active_session().history()[1].context_override(),
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
            state.active_session().history()[0].context_override(),
            ContextOverride::ForcedExclude
        );

        // When handling second press (cursor on pinned entry 1).
        let _result = handle_ignore_selected(&mut state);

        // Then pinned entry 1 is unchanged (still Default).
        assert_eq!(
            state.active_session().history()[1].context_override(),
            ContextOverride::Default
        );
        // And entry 2 (after pinned) gets ForcedExclude.
        assert_eq!(
            state.active_session().history()[2].context_override(),
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
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("pinned").with_pin(PinPosition::Top));
        state.active_session_mut().select_prev_entry();

        // First press - toggles entry 0, advances to entry 1 (pinned).
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(state.active_session().selected_entry_index(), Some(1));

        // When handling second press (cursor on pinned entry 1, at bottom).
        let result = handle_ignore_selected(&mut state);

        // Then pinned entry is unchanged.
        assert_eq!(
            state.active_session().history()[1].context_override(),
            ContextOverride::Default
        );
        // And no commands returned (no-op: pinned at bottom).
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sweep_started_on_toggle_to_default() {
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
            state.active_session().history()[0].context_override(),
            ContextOverride::Default
        );
        // And cursor has advanced.
        assert_eq!(state.active_session().selected_entry_index(), Some(1));
        // And sweep state IS stored (Default is a valid sweep target for un-ignore).
        let sweep = state.active_session_mut().take_ignore_sweep();
        assert_eq!(sweep, Some(ContextOverride::Default));
    }

    #[rstest::rstest]
    fn sweep_skips_collapsed_block_without_mutating() {
        // Given: 1 user, 10 ignored (will collapse), 5 user.
        // The 10 ignored entries form a collapsed block.
        // The sweep should skip the collapsed block entirely — no expansion,
        // no mutation of entries inside.
        use crate::feat::ui::chat_log::visual_item::{
            DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, VisualItem, build_visual_items,
        };

        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("before"));
        for _ in 0..10 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("ignored").with_ignored(true));
        }
        for _ in 0..5 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("after"));
        }

        // Build visual items so the collapsed block exists.
        let items = build_visual_items(
            state.active_session().history(),
            &state.active_session().ui.shown_ignored_blocks,
            PROXIMITY_COUNT,
            DEFAULT_MIN_COLLAPSE_COUNT,
        );
        state.active_session_mut().set_visual_items(items.clone());

        // Verify we have a collapsed block.
        let collapsed = items
            .iter()
            .find(|i| matches!(i, VisualItem::CollapsedIgnoredBlock { .. }));
        assert!(collapsed.is_some(), "should have a collapsed block");

        // Select first user entry.
        let first_vi = items
            .iter()
            .position(|i| matches!(i, VisualItem::Entry(0)))
            .expect("first entry");
        state
            .active_session_mut()
            .set_selected_entry_index(first_vi);

        // First press - toggles entry 0 to ForcedExclude, advances.
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(
            state.active_session().history()[0].context_override(),
            ContextOverride::ForcedExclude
        );

        // Second press - should skip the collapsed block and land on
        // an entry after it.
        let _result = handle_ignore_selected(&mut state);

        // Entries inside the collapsed block should NOT be mutated
        // (they were already ForcedExclude, no change applied).
        // Just verify the block was NOT expanded.
        let block_start_id = state.active_session().history()[1].id.clone();
        assert!(
            !state
                .active_session()
                .ui
                .shown_ignored_blocks
                .contains(&block_start_id),
            "collapsed block should NOT be expanded during sweep"
        );

        // The cursor should have jumped past the collapsed block
        // to one of the 'after' entries.
        let selected = state.active_session().selected_entry();
        assert!(
            selected.is_some(),
            "cursor should be on an entry after the collapsed block"
        );
        let selected_text = selected.expect("entry").text();
        assert!(
            selected_text.starts_with("after"),
            "cursor should be on an 'after' entry, got: {selected_text:?}"
        );
    }

    #[rstest::rstest]
    fn sweep_unignore_propagates_shown_to_new_block() {
        // Given: 1 user, 15 ignored in a shown (expanded) block, 5 user.
        // Sweep un-ignore will bring entries into context, splitting the block.
        // The new forward sub-block should auto-expand.
        use crate::feat::ui::chat_log::visual_item::{
            DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, build_visual_items,
        };

        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("before"));
        for _ in 0..15 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("ignored").with_ignored(true));
        }
        for _ in 0..5 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("after"));
        }

        // Show (expand) the block first.
        let block_start_id = state.active_session().history()[1].id.clone();
        state
            .active_session_mut()
            .ui
            .shown_ignored_blocks
            .insert(block_start_id.clone());

        // Build visual items (now expanded - individual entries).
        let items = build_visual_items(
            state.active_session().history(),
            &state.active_session().ui.shown_ignored_blocks,
            PROXIMITY_COUNT,
            DEFAULT_MIN_COLLAPSE_COUNT,
        );
        state.active_session_mut().set_visual_items(items.clone());

        // Select entry at history index 1 (first ignored entry).
        let vi_idx = items
            .iter()
            .position(|i| {
                matches!(
                    i,
                    crate::feat::ui::chat_log::visual_item::VisualItem::Entry(1)
                )
            })
            .expect("entry at history index 1");
        state.active_session_mut().set_selected_entry_index(vi_idx);

        // First press - toggles entry 1 from ForcedExclude → Default.
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(
            state.active_session().history()[1].context_override(),
            ContextOverride::Default
        );
        // Sweep state is Default (un-ignore sweep).
        assert_eq!(
            state.active_session_mut().take_ignore_sweep(),
            Some(ContextOverride::Default)
        );
        // Restore sweep since we consumed it.
        state
            .active_session_mut()
            .set_ignore_sweep(ContextOverride::Default);

        // Propagation should have shown the forward sub-block.
        // Entry 2 is the start of the forward excluded sub-block.
        let forward_block_id = state.active_session().history()[2].id.clone();
        assert!(
            state
                .active_session()
                .ui
                .shown_ignored_blocks
                .contains(&forward_block_id),
            "forward sub-block should be auto-shown after un-ignore split"
        );
    }

    #[rstest::rstest]
    fn sweep_continues_past_collapsed_block_to_entries_beyond() {
        // Given: 1 user (in-context), 10 ignored (collapsed block), 3 user (in-context).
        // Sweep starts on the first user entry, should continue through the
        // collapsed block and reach the user entries after it.
        use crate::feat::ui::chat_log::visual_item::{
            DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, VisualItem, build_visual_items,
        };

        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("before"));
        for _ in 0..10 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("ignored").with_ignored(true));
        }
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("after1"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("after2"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("after3"));

        // Build visual items.
        let items = build_visual_items(
            state.active_session().history(),
            &state.active_session().ui.shown_ignored_blocks,
            PROXIMITY_COUNT,
            DEFAULT_MIN_COLLAPSE_COUNT,
        );
        state.active_session_mut().set_visual_items(items.clone());

        // Verify layout: Entry(0), CollapsedIgnoredBlock(1, 10), Entry(11), Entry(12), Entry(13).
        let collapsed = items
            .iter()
            .find(|i| matches!(i, VisualItem::CollapsedIgnoredBlock { .. }));
        assert!(collapsed.is_some(), "should have a collapsed block");

        // Select first user entry ("before").
        let first_vi = items
            .iter()
            .position(|i| matches!(i, VisualItem::Entry(0)))
            .expect("first entry");
        state
            .active_session_mut()
            .set_selected_entry_index(first_vi);

        // First press - toggles "before" to ForcedExclude, advances to collapsed block.
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(
            state.active_session().history()[0].context_override(),
            ContextOverride::ForcedExclude
        );

        // Second press - should expand collapsed block, apply override, advance past it.
        let _result = handle_ignore_selected(&mut state);

        // The sweep should have continued — cursor should now be on an entry
        // after the collapsed block (not stuck on it).
        let selected_idx = state.active_session().selected_entry_index();
        assert!(
            selected_idx.is_some(),
            "cursor should have a selection after sweep through block"
        );

        // The selected entry should be one of the "after" entries (history index 11+).
        let selected_entry = state.active_session().selected_entry().expect("entry");
        assert!(
            selected_entry.text().starts_with("after"),
            "cursor should be on an 'after' entry, got: {:?}",
            selected_entry.text()
        );

        // Third press - should continue sweeping the "after" entries.
        let _result = handle_ignore_selected(&mut state);
        // Verify one of the after entries got ForcedExclude.
        let any_after_excluded = state
            .active_session()
            .history()
            .iter()
            .skip(11)
            .any(|e| e.context_override() == ContextOverride::ForcedExclude);
        assert!(
            any_after_excluded,
            "at least one 'after' entry should be ForcedExclude"
        );
    }

    #[rstest::rstest]
    #[rstest::rstest]
    fn sweep_skips_multiple_collapsed_blocks() {
        // Given: 2 user, 10 ignored (block 1), 2 user, 10 ignored (block 2), 5 user.
        // Sweep starts on first user, skips collapsed blocks, processes in-between entries.
        use crate::feat::ui::chat_log::visual_item::{
            DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, VisualItem, build_visual_items,
        };

        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        for _ in 0..10 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("ignored1").with_ignored(true));
        }
        state.active_session_mut().push_entry(ChatEntry::user("c"));
        state.active_session_mut().push_entry(ChatEntry::user("d"));
        for _ in 0..10 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("ignored2").with_ignored(true));
        }
        for _ in 0..5 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("after"));
        }

        // Build visual items.
        let items = build_visual_items(
            state.active_session().history(),
            &state.active_session().ui.shown_ignored_blocks,
            PROXIMITY_COUNT,
            DEFAULT_MIN_COLLAPSE_COUNT,
        );
        state.active_session_mut().set_visual_items(items.clone());

        // Verify two collapsed blocks exist.
        let collapsed_count = items
            .iter()
            .filter(|i| matches!(i, VisualItem::CollapsedIgnoredBlock { .. }))
            .count();
        assert_eq!(collapsed_count, 2, "should have two collapsed blocks");

        // The ignored entries inside the blocks should NOT be mutated.
        let block1_start_id = state.active_session().history()[2].id.clone();

        // Select first entry.
        let first_vi = items
            .iter()
            .position(|i| matches!(i, VisualItem::Entry(0)))
            .expect("first entry");
        state
            .active_session_mut()
            .set_selected_entry_index(first_vi);

        // First press - toggles entry "a" to ForcedExclude.
        let _result = handle_ignore_selected(&mut state);
        assert_eq!(
            state.active_session().history()[0].context_override(),
            ContextOverride::ForcedExclude
        );

        // Second press - cursor on first collapsed block, skips it.
        // Lands on entry "c" and applies ForcedExclude.
        let _result = handle_ignore_selected(&mut state);

        // Entry "b" should NOT be mutated (it was before the block).
        // The block entries should NOT be mutated (skipped).
        for i in 2..=11 {
            assert_eq!(
                state.active_session().history()[i].context_override(),
                ContextOverride::ForcedExclude,
                "ignored entry {i} should still be ForcedExclude (untouched by sweep)"
            );
        }

        // Entry "c" (index 12) should be processed.
        assert_eq!(
            state.active_session().history()[12].context_override(),
            ContextOverride::ForcedExclude,
            "entry 'c' should be ForcedExclude after sweep past first block"
        );

        // The block should NOT be expanded.
        assert!(
            !state
                .active_session()
                .ui
                .shown_ignored_blocks
                .contains(&block1_start_id),
            "first collapsed block should not be expanded"
        );
    }
}

/// Common tail for the x-sweep: persist session and emit one
/// `ContextOverrideChanged` event per entry whose override actually changed.
fn finalize_sweep(
    _state: &mut AppState,
    session_id: crate::protocol::SessionId,
    changed_ids: Vec<crate::protocol::ChatEntryId>,
) -> IntentResult {
    let events: Vec<Event> = changed_ids
        .into_iter()
        .map(|id| {
            Event::ContextOverrideChanged(
                crate::feat::context::protocol::event::ContextOverrideChanged {
                    session_id: session_id.clone(),
                    entry_id: id,
                },
            )
        })
        .collect();
    IntentResult::with_commands_and_events(
        vec![Command::PersistSession(
            crate::feat::session_lifecycle::protocol::command::PersistSession { session_id },
        )],
        events,
    )
}
