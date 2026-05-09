//! Navigation intent validators.
//!
//! Validators for scrolling, tab switching, and edit-input intents.
//! All are infallible — they always succeed.

use nullslop_component::AppState;
use nullslop_protocol::TabDirection;

/// Validates the ScrollUp intent.
pub fn validate_scroll_up(_state: &AppState) {}

/// Validates the ScrollDown intent.
pub fn validate_scroll_down(_state: &AppState) {}

/// Validates the MouseScrollUp intent.
pub fn validate_mouse_scroll_up(_state: &AppState) {}

/// Validates the MouseScrollDown intent.
pub fn validate_mouse_scroll_down(_state: &AppState) {}

/// Validates the ScrollToTop intent.
pub fn validate_scroll_to_top(_state: &AppState) {}

/// Validates the ScrollToBottom intent.
pub fn validate_scroll_to_bottom(_state: &AppState) {}

/// Validates the SwitchTab intent.
pub fn validate_switch_tab(_state: &AppState, _direction: &TabDirection) {}

/// Validates the EditInput intent.
pub fn validate_edit_input(_state: &AppState) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn validate_scroll_up_always_succeeds() {
        let state = AppState::default();
        validate_scroll_up(&state);
    }

    #[rstest::rstest]
    fn validate_scroll_down_always_succeeds() {
        let state = AppState::default();
        validate_scroll_down(&state);
    }

    #[rstest::rstest]
    fn validate_mouse_scroll_up_always_succeeds() {
        let state = AppState::default();
        validate_mouse_scroll_up(&state);
    }

    #[rstest::rstest]
    fn validate_mouse_scroll_down_always_succeeds() {
        let state = AppState::default();
        validate_mouse_scroll_down(&state);
    }

    #[rstest::rstest]
    fn validate_scroll_to_top_always_succeeds() {
        let state = AppState::default();
        validate_scroll_to_top(&state);
    }

    #[rstest::rstest]
    fn validate_scroll_to_bottom_always_succeeds() {
        let state = AppState::default();
        validate_scroll_to_bottom(&state);
    }

    #[rstest::rstest]
    fn validate_switch_tab_always_succeeds() {
        let state = AppState::default();
        validate_switch_tab(&state, &TabDirection::Next);
    }

    #[rstest::rstest]
    fn validate_edit_input_always_succeeds() {
        let state = AppState::default();
        validate_edit_input(&state);
    }
}
