//! Pinned panel intent validators.
//!
//! Validators for pinned panel navigation, toggling, and pin/unpin actions.
//! Navigation and toggle intents are infallible; pin/unpin actions are fallible.

use nullslop_component::AppState;
use wherror::Error;

// --- Fallible validators ---

/// Errors from validating pinned panel pin/unpin intents.
#[derive(Debug, Error)]
#[error(debug)]
pub enum PinnedPanelActionError {
    /// No pinned entry is selected.
    NoSelection,
    /// No pinned entries exist.
    Empty,
}

/// Validates the PinnedPanelUnpin intent.
///
/// Returns an error if there are no pinned entries or no entry is selected.
pub fn validate_pinned_panel_unpin(state: &AppState) -> Result<(), PinnedPanelActionError> {
    validate_pin_action(state)
}

/// Validates the PinnedPanelPinTop intent.
pub fn validate_pinned_panel_pin_top(state: &AppState) -> Result<(), PinnedPanelActionError> {
    validate_pin_action(state)
}

/// Validates the PinnedPanelPinBottom intent.
pub fn validate_pinned_panel_pin_bottom(state: &AppState) -> Result<(), PinnedPanelActionError> {
    validate_pin_action(state)
}

/// Validates the PinnedPanelPinRelative intent.
pub fn validate_pinned_panel_pin_relative(
    state: &AppState,
) -> Result<(), PinnedPanelActionError> {
    validate_pin_action(state)
}

/// Validates the PinnedPanelPinCycle intent.
pub fn validate_pinned_panel_pin_cycle(state: &AppState) -> Result<(), PinnedPanelActionError> {
    validate_pin_action(state)
}

/// Shared validation logic for all pin/unpin actions.
///
/// Checks that there are pinned entries and one is selected.
fn validate_pin_action(state: &AppState) -> Result<(), PinnedPanelActionError> {
    if state.sorted_pinned_ids().is_empty() {
        return Err(PinnedPanelActionError::Empty);
    }
    if state.pinned_panel.selected_id().is_none() {
        return Err(PinnedPanelActionError::NoSelection);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nullslop_protocol::{ChatEntry, PinPosition};

    use super::*;

    fn state_with_selected_pin(text: &str, position: PinPosition) -> AppState {
        let mut state = AppState::default();
        let entry_id = {
            let index = state
                .active_session_mut()
                .push_entry(ChatEntry::user(text));
            state.active_session().history()[index].id.clone()
        };
        state.active_session_mut().pin_entry(&entry_id, position);
        state.pinned_panel.select_by_id(entry_id);
        state
    }

    fn state_with_unselected_pin(text: &str, position: PinPosition) -> AppState {
        let mut state = AppState::default();
        let entry_id = {
            let index = state
                .active_session_mut()
                .push_entry(ChatEntry::user(text));
            state.active_session().history()[index].id.clone()
        };
        state.active_session_mut().pin_entry(&entry_id, position);
        state
    }

    // --- PinnedPanelUnpin tests ---

    #[rstest::rstest]
    fn unpin_succeeds_with_selected_pinned_entry() {
        // Given a state with a pinned entry that is selected.
        let state = state_with_selected_pin("hello", PinPosition::Top);

        // When validating unpin.
        let result = validate_pinned_panel_unpin(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn unpin_fails_with_no_pinned_entries() {
        // Given a state with no pinned entries.
        let state = AppState::default();

        // When validating unpin.
        let result = validate_pinned_panel_unpin(&state);

        // Then it returns Empty error.
        assert!(matches!(result, Err(PinnedPanelActionError::Empty)));
    }

    #[rstest::rstest]
    fn unpin_fails_with_no_selection() {
        // Given a state with pinned entries but nothing selected.
        let state = state_with_unselected_pin("hello", PinPosition::Top);

        // When validating unpin.
        let result = validate_pinned_panel_unpin(&state);

        // Then it returns NoSelection error.
        assert!(matches!(result, Err(PinnedPanelActionError::NoSelection)));
    }

    // --- PinnedPanelPinTop tests ---

    #[rstest::rstest]
    fn pin_top_succeeds_with_selected_entry() {
        // Given a state with a pinned entry that is selected.
        let state = state_with_selected_pin("hello", PinPosition::Top);

        // When validating pin top.
        let result = validate_pinned_panel_pin_top(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn pin_top_fails_with_no_pinned_entries() {
        // Given a state with no pinned entries.
        let state = AppState::default();

        // When validating pin top.
        let result = validate_pinned_panel_pin_top(&state);

        // Then it returns Empty error.
        assert!(matches!(result, Err(PinnedPanelActionError::Empty)));
    }

    // --- PinnedPanelPinBottom tests ---

    #[rstest::rstest]
    fn pin_bottom_succeeds_with_selected_entry() {
        // Given a state with a pinned entry that is selected.
        let state = state_with_selected_pin("hello", PinPosition::Top);

        // When validating pin bottom.
        let result = validate_pinned_panel_pin_bottom(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn pin_bottom_fails_with_no_selection() {
        // Given a state with pinned entries but nothing selected.
        let state = state_with_unselected_pin("hello", PinPosition::Top);

        // When validating pin bottom.
        let result = validate_pinned_panel_pin_bottom(&state);

        // Then it returns NoSelection error.
        assert!(matches!(result, Err(PinnedPanelActionError::NoSelection)));
    }

    // --- PinnedPanelPinRelative tests ---

    #[rstest::rstest]
    fn pin_relative_succeeds_with_selected_entry() {
        // Given a state with a pinned entry that is selected.
        let state = state_with_selected_pin("hello", PinPosition::Relative);

        // When validating pin relative.
        let result = validate_pinned_panel_pin_relative(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn pin_relative_fails_with_no_pinned_entries() {
        // Given a state with no pinned entries.
        let state = AppState::default();

        // When validating pin relative.
        let result = validate_pinned_panel_pin_relative(&state);

        // Then it returns Empty error.
        assert!(matches!(result, Err(PinnedPanelActionError::Empty)));
    }

    // --- PinnedPanelPinCycle tests ---

    #[rstest::rstest]
    fn pin_cycle_succeeds_with_selected_entry() {
        // Given a state with a pinned entry that is selected.
        let state = state_with_selected_pin("hello", PinPosition::Top);

        // When validating pin cycle.
        let result = validate_pinned_panel_pin_cycle(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn pin_cycle_fails_with_no_selection() {
        // Given a state with pinned entries but nothing selected.
        let state = state_with_unselected_pin("hello", PinPosition::Top);

        // When validating pin cycle.
        let result = validate_pinned_panel_pin_cycle(&state);

        // Then it returns NoSelection error.
        assert!(matches!(result, Err(PinnedPanelActionError::NoSelection)));
    }
}
