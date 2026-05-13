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

/// Validates the ToggleKeymapScopeFilter intent.
pub fn validate_toggle_keymap_scope_filter(_state: &AppState) {}

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
        .cloned()
        .ok_or(PickerConfirmError::NoActivePicker)?;

    let has_selection = match kind {
        PickerKind::Provider => state.provider.provider_picker.selected_item().is_some(),
        PickerKind::ContextAssembly => state
            .frontend
            .context_strategy_picker
            .selected_item()
            .is_some(),
        PickerKind::Keymap => state.frontend.keymap_picker.selected_item().is_some(),
        PickerKind::Session => state.frontend.session_picker.selected_item().is_some(),
        PickerKind::Persona => state.frontend.persona_picker.selected_item().is_some(),
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
    use super::*;

    use crate::common::app_state::FocusScope;

    // --- PickerConfirm tests ---

    #[rstest::rstest]
    fn picker_confirm_fails_with_no_active_picker() {
        // Given a state with no active picker.
        let state = AppState::default();

        // When validating picker confirm.
        let result = validate_picker_confirm(&state);

        // Then it returns NoActivePicker error.
        assert!(matches!(result, Err(PickerConfirmError::NoActivePicker)));
    }

    #[rstest::rstest]
    fn picker_confirm_fails_with_no_selection() {
        // Given a state with an active picker but no items.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker { kind: PickerKind::Provider });

        // When validating picker confirm.
        let result = validate_picker_confirm(&state);

        // Then it returns NoSelection error.
        assert!(matches!(result, Err(PickerConfirmError::NoSelection)));
    }

    #[rstest::rstest]
    fn picker_confirm_succeeds_with_provider_selection() {
        // Given a state with an active provider picker and items.
        use crate::protocol::PickerEntry;
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker { kind: PickerKind::Provider });
        state.provider.provider_picker.set_items(vec![PickerEntry {
            provider_id: "test/test-model".to_owned(),
            name: "test".to_owned(),
            provider_name: "test".to_owned(),
            backend: "openai".to_owned(),
            model: "Test Model".to_owned(),
            is_alias: false,
            alias_target: None,
            is_available: true,
            is_remote: false,
            is_active: false,
        }]);

        // When validating picker confirm.
        let result = validate_picker_confirm(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn picker_confirm_succeeds_with_keymap_selection() {
        // Given a state with an active keymap picker and items.
        use crate::protocol::KeymapEntry;
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker { kind: PickerKind::Keymap });
        state.frontend.keymap_picker.set_items(vec![KeymapEntry {
            key_sequence: "q".to_owned(),
            description: "quit".to_owned(),
            scope: "normal".to_owned(),
            category: "general".to_owned(),
            command: crate::protocol::Intent::Quit,
            search_text: "q quit".to_owned(),
        }]);

        // When validating picker confirm.
        let result = validate_picker_confirm(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn picker_confirm_succeeds_with_session_selection() {
        // Given a state with an active session picker and items.
        use crate::protocol::SessionEntry;
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker { kind: PickerKind::Session });
        state.frontend.session_picker.set_items(vec![SessionEntry {
            session_id: crate::protocol::SessionId::new(),
            title: "Test Session".to_owned(),
            updated_at: jiff::Timestamp::now(),
            byte_offset: 0,
        }]);

        // When validating picker confirm.
        let result = validate_picker_confirm(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn picker_confirm_succeeds_with_context_assembly_selection() {
        // Given a state with an active context strategy picker and items.
        use crate::protocol::StrategyEntry;
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker { kind: PickerKind::ContextAssembly });
        state
            .frontend
            .context_strategy_picker
            .set_items(vec![StrategyEntry {
                strategy_id: crate::protocol::PromptStrategyId::passthrough(),
                name: "passthrough".to_owned(),
                description: "No processing".to_owned(),
                is_active: false,
            }]);

        // When validating picker confirm.
        let result = validate_picker_confirm(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    // --- OpenPicker tests ---

    #[rstest::rstest]
    fn open_picker_succeeds_when_no_picker_active() {
        // Given a state with no active picker.
        let state = AppState::default();

        // When validating open provider picker.
        let result = validate_open_picker(&state, &PickerKind::Provider);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn open_picker_fails_when_already_in_picker() {
        // Given a state with an active picker.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker { kind: PickerKind::Keymap });

        // When validating open provider picker.
        let result = validate_open_picker(&state, &PickerKind::Provider);

        // Then it returns AlreadyInPicker error.
        assert!(matches!(result, Err(OpenPickerError::AlreadyInPicker)));
    }
}
