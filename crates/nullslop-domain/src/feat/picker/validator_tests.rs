use crate::common::app_state::AppState;
use crate::common::app_state::FocusScope;
use crate::feat::picker::validator::{
    OpenPickerError, PickerConfirmError, validate_open_picker, validate_picker_confirm,
};
use crate::feat::theme::default_theme;
use crate::protocol::PickerKind;

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
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });

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
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.set_items(vec![PickerEntry {
        provider_id: "test/test-model".to_owned(),
        name: "test".to_owned(),
        provider_name: "test".to_owned(),
        backend: "openai".to_owned(),
        model: "Test Model".to_owned(),
        search_text: "Test Model test".to_owned(),
        is_alias: false,
        alias_target: None,
        is_available: true,
        is_remote: false,
        is_active: false,
        theme: default_theme(),
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
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Keymap,
    });
    state.frontend.keymap_picker.set_items(vec![KeymapEntry {
        key_sequence: "q".to_owned(),
        description: "quit".to_owned(),
        scope: "normal".to_owned(),
        category: "general".to_owned(),
        command: crate::protocol::Intent::Quit,
        search_text: "q quit".to_owned(),
        theme: state.frontend.theme.clone(),
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
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Session,
    });
    state.frontend.session_picker.set_items(vec![SessionEntry {
        session_id: crate::protocol::SessionId::new(),
        title: "Test Session".to_owned(),
        updated_at: jiff::Timestamp::now(),
        theme: default_theme(),
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
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::ContextAssembly,
    });
    state
        .frontend
        .context_strategy_picker
        .set_items(vec![StrategyEntry {
            strategy_id: crate::protocol::PromptStrategyId::passthrough(),
            name: "passthrough".to_owned(),
            description: "No processing".to_owned(),
            is_active: false,
            theme: default_theme(),
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
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Keymap,
    });

    // When validating open provider picker.
    let result = validate_open_picker(&state, &PickerKind::Provider);

    // Then it returns AlreadyInPicker error.
    assert!(matches!(result, Err(OpenPickerError::AlreadyInPicker)));
}

// --- Regression: picker confirm then reopen ---

#[rstest::rstest]
fn picker_confirm_then_reopen_succeeds() {
    // Given a state with an active provider picker and a selected item.
    use crate::protocol::PickerEntry;
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.set_items(vec![PickerEntry {
        provider_id: "test/model".to_owned(),
        name: "test".to_owned(),
        provider_name: "test".to_owned(),
        backend: "openai".to_owned(),
        model: "Test".to_owned(),
        search_text: "Test test".to_owned(),
        is_alias: false,
        alias_target: None,
        is_available: true,
        is_remote: false,
        is_active: false,
        theme: default_theme(),
    }]);

    // When confirming (simulates scope pop) then reopening.
    state.frontend.scope_stack.pop();
    let result = validate_open_picker(&state, &PickerKind::Provider);

    // Then reopening succeeds (picker was properly cleared).
    assert!(result.is_ok());
}
