//! Picker intent validators.
//!
//! Validators for picker navigation, confirmation, and opening intents.
//! Most are infallible; picker confirm and open picker are fallible.

use crate::common::app_state::AppState;
use crate::protocol::PickerKind;
use wherror::Error;

// --- Infallible validators ---

/// Validates the PickerInsertChar intent.
pub fn validate_picker_insert_char(_state: &AppState, _ch: char) {}

/// Validates the PickerBackspace intent.
pub fn validate_picker_backspace(_state: &AppState) {}

/// Validates the PickerMoveUp intent.
pub fn validate_picker_move_up(_state: &AppState) {}

/// Validates the PickerMoveDown intent.
pub fn validate_picker_move_down(_state: &AppState) {}

/// Validates the PickerMoveCursorLeft intent.
pub fn validate_picker_move_cursor_left(_state: &AppState) {}

/// Validates the PickerMoveCursorRight intent.
pub fn validate_picker_move_cursor_right(_state: &AppState) {}

// --- Fallible validators ---

/// Errors from validating a PickerConfirm intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum PickerConfirmError {
    /// No picker is active.
    NoActivePicker,
    /// No item is selected in the picker.
    NoSelection,
}

/// Validates the PickerConfirm intent.
///
/// Returns an error if no picker is active or no item is selected.
///
/// # Errors
///
/// Returns an error if no picker is active or no item is selected.
pub fn validate_picker_confirm(state: &AppState) -> Result<(), PickerConfirmError> {
    let kind = state
        .frontend
        .scope_stack
        .picker_kind()
        .copied()
        .ok_or(PickerConfirmError::NoActivePicker)?;

    let has_selection = match kind {
        PickerKind::Provider => state.provider.provider_picker.selected_item().is_some(),
        PickerKind::Session => state.frontend.session_picker.selected_item().is_some(),
        PickerKind::Persona => state.frontend.persona_picker.selected_item().is_some(),
        PickerKind::Theme => state.frontend.theme_picker.selected_item().is_some(),
        PickerKind::SessionLifecycle => state
            .frontend
            .session_lifecycle_picker
            .selected_item()
            .is_some(),
        PickerKind::Workflow => state.frontend.workflow_picker.selected_item().is_some(),
        PickerKind::Judge => state.frontend.judge_picker.selected_item().is_some(),
        PickerKind::CompactionModel => state
            .frontend
            .compaction_model_picker
            .selected_item()
            .is_some(),
    };

    if has_selection {
        Ok(())
    } else {
        Err(PickerConfirmError::NoSelection)
    }
}

/// Errors from validating an OpenPicker intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum OpenPickerError {
    /// Already in picker mode.
    AlreadyInPicker,
}

/// Validates the OpenPicker intent.
///
/// Returns an error if a picker is already active.
///
/// # Errors
///
/// Returns an error if a picker is already active.
pub fn validate_open_picker(state: &AppState, _kind: &PickerKind) -> Result<(), OpenPickerError> {
    if state.frontend.scope_stack.is_picker() {
        return Err(OpenPickerError::AlreadyInPicker);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_state::FocusScope;

    #[rstest::rstest]
    fn validate_picker_confirm_rejects_no_active_picker() {
        // Kills: replace validate_picker_confirm with Ok(()).
        // If the validator always returned Ok, confirming with no picker would be allowed.
        let state = AppState::default();

        let result = validate_picker_confirm(&state);

        assert!(result.is_err(), "should reject confirm when no picker is active");
    }

    #[rstest::rstest]
    fn validate_open_picker_rejects_when_already_in_picker() {
        // Kills: replace validate_open_picker with Ok(()).
        // If the validator always returned Ok, nested pickers would be allowed.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .push(FocusScope::Picker { kind: PickerKind::Provider });

        let result = validate_open_picker(&state, &PickerKind::Session);

        assert!(result.is_err(), "should reject opening a second picker");
    }

    #[rstest::rstest]
    fn validate_open_picker_allows_when_no_picker_active() {
        // Verifies the positive case — opening a picker when none is active.
        let state = AppState::default();

        let result = validate_open_picker(&state, &PickerKind::Provider);

        assert!(result.is_ok(), "should allow opening picker when none is active");
    }
}
