//! Picker intent validators.
//!
//! Validators for picker navigation, confirmation, and opening intents.
//! Most are infallible; picker confirm and open picker are fallible.

use crate::common::app_state::AppState;
use crate::feat::ui::picker_states::PickerExt;
use crate::protocol::PickerKind;
use wherror::Error;

/// Validates the PickerInsertChar intent.
pub fn validate_picker_insert_char(_state: &AppState, _ch: char) {}

/// Validates the PickerBackspace intent.
pub fn validate_picker_backspace(_state: &AppState) {}

/// Validates the PickerMoveUp intent.
pub fn validate_picker_move_up(_state: &AppState) {}

/// Validates the PickerMoveDown intent.
pub fn validate_picker_move_down(_state: &AppState) {}

/// Validates the PickerPageUp intent.
pub fn validate_picker_page_up(_state: &AppState) {}

/// Validates the PickerPageDown intent.
pub fn validate_picker_page_down(_state: &AppState) {}

/// Validates the PickerMoveCursorLeft intent.
pub fn validate_picker_move_cursor_left(_state: &AppState) {}

/// Validates the PickerMoveCursorRight intent.
pub fn validate_picker_move_cursor_right(_state: &AppState) {}

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
        .picker_kind()
        .ok_or(PickerConfirmError::NoActivePicker)?;

    let has_selection = match kind {
        PickerKind::Project => state.frontend.project_picker().selected_item().is_some(),
        PickerKind::McpServer => state.frontend.mcp_server_picker().selected_item().is_some(),
        // Retired: no picker state, so it can never have a selection.
        PickerKind::CompactionModel => false,
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
    if state.frontend.is_picker() {
        return Err(OpenPickerError::AlreadyInPicker);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::common::app_state::AppState;
    use jinn_slices::FocusScope;

    #[rstest::rstest]
    fn validate_picker_confirm_rejects_no_active_picker() {
        // If the validator always returned Ok, confirming with no picker would be allowed.
        let state = AppState::default_with_scope_focus();

        let result = validate_picker_confirm(&state);

        assert!(
            result.is_err(),
            "should reject confirm when no picker is active"
        );
    }

    #[rstest::rstest]
    fn validate_open_picker_rejects_when_already_in_picker() {
        // If the validator always returned Ok, nested pickers would be allowed.
        let state = AppState::default_with_scope_focus();
        state.frontend.scope_push(FocusScope::Picker {
            kind: PickerKind::Project,
        });

        let result = validate_open_picker(&state, &PickerKind::Project);

        assert!(result.is_err(), "should reject opening a second picker");
    }

    #[rstest::rstest]
    fn validate_open_picker_allows_when_no_picker_active() {
        // Verifies the positive case - opening a picker when none is active.
        let state = AppState::default_with_scope_focus();

        let result = validate_open_picker(&state, &PickerKind::Project);

        assert!(
            result.is_ok(),
            "should allow opening picker when none is active"
        );
    }

    #[rstest::rstest]
    fn validate_picker_confirm_accepts_project_with_selection() {
        // If the selection gate were broken, confirming with a selection would
        // be rejected.
        let mut state = AppState::default_with_scope_focus();
        state
            .frontend
            .project_picker_mut()
            .set_items(jinn_picker::make_items_with_hooks(
                vec![test_project("/workspace/demo")],
                jinn_picker::PickerItemHooks::new()
                    .search(|entry: &jinn_project_msg::ProjectEntry| entry.display.clone()),
            ));
        state.frontend.scope_push(FocusScope::Picker {
            kind: PickerKind::Project,
        });

        let result = validate_picker_confirm(&state);

        assert!(
            result.is_ok(),
            "should accept confirm when a project entry is selected"
        );
    }

    #[rstest::rstest]
    fn validate_picker_confirm_rejects_project_without_selection() {
        // If the selection gate were broken, confirming with no selection
        // would be allowed.
        let state = AppState::default_with_scope_focus();
        // No entries set, so no selection.
        state.frontend.scope_push(FocusScope::Picker {
            kind: PickerKind::Project,
        });

        let result = validate_picker_confirm(&state);

        assert!(
            result.is_err(),
            "should reject confirm when no project entry is selected"
        );
    }

    /// A project entry for the selection-gate tests above.
    fn test_project(path: &str) -> jinn_project_msg::ProjectEntry {
        jinn_project_msg::ProjectEntry::new(
            std::path::PathBuf::from(path),
            jinn_theme::default_theme(),
        )
    }
}
