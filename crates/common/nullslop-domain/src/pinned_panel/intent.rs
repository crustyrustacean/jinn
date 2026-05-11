//! Pinned panel intent handlers.
//!
//! Handles all 11 pinned-panel intents:
//!
//! - **Toggle/Open/Close** — set TUI signals to show/hide the panel.
//! - **SelectDown/SelectUp** — move selection within the pinned entries list.
//! - **Unpin** — remove the selected entry's pin.
//! - **Pin Top/Bottom/Relative** — set the selected entry's pin position.
//! - **PinCycle** — rotate the selected entry's position (Top → Bottom → Relative → Top).

use crate::component::AppState;
use crate::component::app_state::pin_sort_key;
use crate::protocol::context::{PinChatEntry, UnpinChatEntry};
use crate::protocol::{ChatEntryId, Command, IntentResult, PinPosition, SessionId};

use super::validator;

// --- Toggle / Open / Close ---

/// Handles `PinnedPanelToggle` — sets the toggle signal.
pub fn handle_toggle(state: &mut AppState) -> IntentResult {
    state.frontend.tui_signals.pinned_pane_toggle = true;
    IntentResult::empty()
}

/// Handles `PinnedPanelOpen` — sets the open signal.
pub fn handle_open(state: &mut AppState) -> IntentResult {
    state.frontend.tui_signals.pinned_pane_open = true;
    IntentResult::empty()
}

/// Handles `PinnedPanelClose` — sets the close signal.
pub fn handle_close(state: &mut AppState) -> IntentResult {
    state.frontend.tui_signals.pinned_pane_close = true;
    IntentResult::empty()
}

// --- Selection ---

/// Handles `PinnedPanelSelectDown` — moves selection to the next pinned entry.
pub fn handle_select_down(state: &mut AppState) -> IntentResult {
    let sorted_ids = state.sorted_pinned_ids();
    state.frontend.pinned_panel.select_next(&sorted_ids);
    IntentResult::empty()
}

/// Handles `PinnedPanelSelectUp` — moves selection to the previous pinned entry.
pub fn handle_select_up(state: &mut AppState) -> IntentResult {
    let sorted_ids = state.sorted_pinned_ids();
    state.frontend.pinned_panel.select_prev(&sorted_ids);
    IntentResult::empty()
}

// --- Pin / Unpin ---

/// Handles `PinnedPanelUnpin` — unpins the selected entry.
pub fn handle_pinned_panel_unpin(state: &mut AppState) -> IntentResult {
    if validator::validate_pinned_panel_unpin(state).is_err() {
        return IntentResult::empty();
    }

    if let Some((session_id, entry_id)) = resolve_selected_entry_id(state) {
        IntentResult::with_commands(vec![Command::UnpinChatEntry {
            payload: UnpinChatEntry {
                session_id,
                entry_id,
            },
        }])
    } else {
        IntentResult::empty()
    }
}

/// Handles `PinnedPanelPinTop/Bottom/Relative` — sets the selected entry's pin position.
pub fn handle_pinned_panel_pin(state: &mut AppState, position: PinPosition) -> IntentResult {
    if validator::validate_pinned_panel_pin_top(state).is_err() {
        return IntentResult::empty();
    }

    if let Some((session_id, entry_id)) = resolve_selected_entry_id(state) {
        IntentResult::with_commands(vec![Command::PinChatEntry {
            payload: PinChatEntry {
                session_id,
                entry_id,
                position,
            },
        }])
    } else {
        IntentResult::empty()
    }
}

/// Handles `PinnedPanelPinCycle` — rotates the selected entry's pin position.
pub fn handle_pinned_panel_pin_cycle(state: &mut AppState) -> IntentResult {
    if validator::validate_pinned_panel_pin_cycle(state).is_err() {
        return IntentResult::empty();
    }

    let sorted_ids = state.sorted_pinned_ids();
    let index = state.frontend.pinned_panel.selection_index(&sorted_ids);

    let mut pinned = state.active_session().pinned_entries();
    pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));

    let Some(entry) = pinned.get(index) else {
        return IntentResult::empty();
    };

    let current = entry.pin_position.unwrap_or(PinPosition::Relative);
    let next = cycle_position(current);
    let session_id = state.session.active_session.clone();
    let entry_id = entry.id.clone();

    IntentResult::with_commands(vec![Command::PinChatEntry {
        payload: PinChatEntry {
            session_id,
            entry_id,
            position: next,
        },
    }])
}

// --- Helpers ---

/// Resolves the currently selected pinned entry to its session and entry IDs.
fn resolve_selected_entry_id(state: &AppState) -> Option<(SessionId, ChatEntryId)> {
    let sorted_ids = state.sorted_pinned_ids();
    let index = state.frontend.pinned_panel.selection_index(&sorted_ids);
    let session_id = state.session.active_session.clone();

    let mut pinned = state.active_session().pinned_entries();
    pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));

    let entry = pinned.get(index)?;
    Some((session_id, entry.id.clone()))
}

/// Cycles a pin position to the next value in the rotation: Top → Bottom → Relative → Top.
fn cycle_position(pos: PinPosition) -> PinPosition {
    match pos {
        PinPosition::Top => PinPosition::Bottom,
        PinPosition::Bottom => PinPosition::Relative,
        PinPosition::Relative => PinPosition::Top,
    }
}

#[cfg(test)]
mod tests {
    use crate::component::AppState;
    use crate::protocol::{ChatEntry, Command, PinPosition};

    use super::*;

    fn state_with_pinned(count: usize) -> AppState {
        let mut state = AppState::default();
        let mut ids = vec![];
        for i in 0..count {
            let entry = ChatEntry::user(format!("entry {i}"));
            let entry_id = entry.id.clone();
            state.active_session_mut().push_entry(entry);
            ids.push(entry_id);
        }
        for id in &ids {
            state.active_session_mut().pin_entry(id, PinPosition::Top);
        }
        // Select the first pinned entry.
        if let Some(first_id) = ids.first() {
            state.frontend.pinned_panel.select_by_id(first_id.clone());
        }
        state
    }

    #[rstest::rstest]
    fn pinned_panel_toggle_sets_signal() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling toggle.
        let result = handle_toggle(&mut state);

        // Then the toggle signal is set.
        assert!(state.frontend.tui_signals.pinned_pane_toggle);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pinned_panel_open_sets_signal() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling open.
        let result = handle_open(&mut state);

        // Then the open signal is set.
        assert!(state.frontend.tui_signals.pinned_pane_open);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pinned_panel_close_sets_signal() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling close.
        let result = handle_close(&mut state);

        // Then the close signal is set.
        assert!(state.frontend.tui_signals.pinned_pane_close);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pinned_panel_select_down_moves_selection() {
        // Given a state with 3 pinned entries.
        let mut state = state_with_pinned(3);

        // When handling select down.
        let result = handle_select_down(&mut state);

        // Then selection moved.
        let sorted_ids = state.sorted_pinned_ids();
        assert_eq!(
            state.frontend.pinned_panel.selected_id(),
            Some(&sorted_ids[1])
        );
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pinned_panel_select_up_moves_selection() {
        // Given a state with 3 pinned entries at index 1.
        let mut state = state_with_pinned(3);
        let sorted_ids = state.sorted_pinned_ids();
        state.frontend.pinned_panel.select_next(&sorted_ids);

        // When handling select up.
        let result = handle_select_up(&mut state);

        // Then selection moved back.
        let sorted_ids = state.sorted_pinned_ids();
        assert_eq!(
            state.frontend.pinned_panel.selected_id(),
            Some(&sorted_ids[0])
        );
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pinned_panel_unpin_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(2);

        // When handling unpin.
        let result = handle_pinned_panel_unpin(&mut state);

        // Then an UnpinChatEntry command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::UnpinChatEntry { .. }))
        );
    }

    #[rstest::rstest]
    fn pinned_panel_unpin_noop_when_empty() {
        // Given a state with no pinned entries.
        let mut state = AppState::default();

        // When handling unpin.
        let result = handle_pinned_panel_unpin(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pinned_panel_pin_top_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(1);

        // When handling pin top.
        let result = handle_pinned_panel_pin(&mut state, PinPosition::Top);

        // Then a PinChatEntry command with Top is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry { payload } => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Top));
    }

    #[rstest::rstest]
    fn pinned_panel_pin_bottom_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(1);

        // When handling pin bottom.
        let result = handle_pinned_panel_pin(&mut state, PinPosition::Bottom);

        // Then a PinChatEntry command with Bottom is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry { payload } => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Bottom));
    }

    #[rstest::rstest]
    fn pinned_panel_pin_relative_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(1);

        // When handling pin relative.
        let result = handle_pinned_panel_pin(&mut state, PinPosition::Relative);

        // Then a PinChatEntry command with Relative is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry { payload } => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Relative));
    }

    #[rstest::rstest]
    fn pinned_panel_pin_cycle_rotates_top_to_bottom() {
        // Given a pinned entry at Top.
        let mut state = AppState::default();
        let entry = ChatEntry::user("entry");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Top);
        let sorted_ids = state.sorted_pinned_ids();
        state
            .frontend
            .pinned_panel
            .select_by_id(sorted_ids[0].clone());

        // When handling pin cycle.
        let result = handle_pinned_panel_pin_cycle(&mut state);

        // Then a PinChatEntry command with Bottom is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry { payload } => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Bottom));
    }

    #[rstest::rstest]
    fn pinned_panel_pin_cycle_noop_when_empty() {
        // Given a state with no pinned entries.
        let mut state = AppState::default();

        // When handling pin cycle.
        let result = handle_pinned_panel_pin_cycle(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pinned_panel_pin_top_noop_when_no_selection() {
        // Given a state with pinned entries but no selection.
        let mut state = AppState::default();
        let entry = ChatEntry::user("entry");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Top);
        // Don't select anything.

        // When handling pin top.
        let result = handle_pinned_panel_pin(&mut state, PinPosition::Top);

        // Then no commands.
        assert!(result.commands.is_empty());
    }
}
