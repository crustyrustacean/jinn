//! Picker intent handlers — navigation, filtering, confirmation, and scope toggling.
//!
//! Handles all picker intents: open, insert char, backspace, confirm, move up/down,
//! cursor movement, and keymap scope filter toggle. The `handle_picker_confirm`
//! function returns `(IntentResult, Option<Intent>)` to allow the caller
//! (`nullslop-intent`) to re-dispatch keymap intents without creating a circular
//! dependency.

use crate::common::app_state::AppState;
use crate::common::app_state::FocusScope;
use crate::feat::context::protocol::command::LoadPersonaPickerEntries;
use crate::feat::context::protocol::command::{
    LoadContextStrategyPickerEntries, SwitchPromptStrategy,
};
use crate::feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
use crate::feat::provider::protocol::command::{LoadProviderPickerEntries, ProviderSwitch};
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::protocol::{Command, Intent, IntentResult, PickerKind};

use super::validator;

/// Maximum number of visible result rows for picker scroll clamping.
const PICKER_MAX_VISIBLE: usize = 100;

/// Opens a picker of the given kind. Sets mode to Picker and optionally
/// requests picker entries from the actor system.
pub fn handle_open_picker(state: &mut AppState, kind: PickerKind) -> IntentResult {
    if validator::validate_open_picker(state, &kind).is_err() {
        return IntentResult::empty();
    }

    state.frontend.scope_stack.push(FocusScope::Picker { kind });

    match kind {
        PickerKind::Provider => {
            state.provider.provider_picker.reset();
        }
        PickerKind::ContextAssembly => {
            state.frontend.context_strategy_picker.reset();
        }
        PickerKind::Keymap => {
            state.frontend.keymap_picker.reset();
            state.frontend.keymap_picker_show_all = false;
        }
        PickerKind::Session => {
            state.frontend.session_picker.reset();
        }
        PickerKind::Persona => {
            state.frontend.persona_picker.reset();
        }
    }

    match kind {
        PickerKind::Provider => {
            IntentResult::with_commands(vec![Command::LoadProviderPickerEntries {
                payload: LoadProviderPickerEntries,
            }])
        }
        PickerKind::Session => {
            IntentResult::with_commands(vec![Command::LoadSessionPickerEntries {
                payload: LoadSessionPickerEntries,
            }])
        }
        PickerKind::ContextAssembly => {
            IntentResult::with_commands(vec![Command::LoadContextStrategyPickerEntries {
                payload: LoadContextStrategyPickerEntries,
            }])
        }
        PickerKind::Persona => {
            IntentResult::with_commands(vec![Command::LoadPersonaPickerEntries {
                payload: LoadPersonaPickerEntries,
            }])
        }
        PickerKind::Keymap => IntentResult::empty(),
    }
}

/// Inserts a character into the active picker's filter.
pub fn handle_insert_char(state: &mut AppState, ch: char) -> IntentResult {
    validator::validate_picker_insert_char(state, ch);
    if let Some(picker) = state.active_picker_ops() {
        picker.insert_char(ch);
    }
    IntentResult::empty()
}

/// Removes the last character from the active picker's filter.
pub fn handle_backspace(state: &mut AppState) -> IntentResult {
    validator::validate_picker_backspace(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.backspace();
    }
    IntentResult::empty()
}

/// Confirms the active picker selection.
///
/// Returns `(IntentResult, Option<Intent>)`. For Provider, ContextAssembly, and
/// Session pickers, the second element is `None`. For Keymap picker, returns
/// `(IntentResult::empty(), Some(selected_intent))` so the caller can re-dispatch.
pub fn handle_picker_confirm(state: &mut AppState) -> (IntentResult, Option<Intent>) {
    if validator::validate_picker_confirm(state).is_err() {
        return (IntentResult::empty(), None);
    }

    match state.frontend.scope_stack.picker_kind().copied() {
        Some(PickerKind::Provider) => (confirm_provider(state), None),
        Some(PickerKind::ContextAssembly) => (confirm_strategy(state), None),
        Some(PickerKind::Keymap) => confirm_keymap(state),
        Some(PickerKind::Session) => (confirm_session(state), None),
        Some(PickerKind::Persona) => (confirm_persona(state), None),
        None => (IntentResult::empty(), None),
    }
}

/// Moves the selection up in the active picker.
pub fn handle_move_up(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_up(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_up(PICKER_MAX_VISIBLE);
    }
    IntentResult::empty()
}

/// Moves the selection down in the active picker.
pub fn handle_move_down(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_down(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_down(PICKER_MAX_VISIBLE);
    }
    IntentResult::empty()
}

/// Moves the filter cursor left in the active picker.
pub fn handle_move_cursor_left(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_cursor_left(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_cursor_left();
    }
    IntentResult::empty()
}

/// Moves the filter cursor right in the active picker.
pub fn handle_move_cursor_right(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_cursor_right(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_cursor_right();
    }
    IntentResult::empty()
}

/// Toggles the keymap scope filter between showing all entries and the
/// current scope's entries only.
pub fn handle_toggle_keymap_scope_filter(state: &mut AppState) -> IntentResult {
    validator::validate_toggle_keymap_scope_filter(state);

    state.frontend.keymap_picker_show_all = !state.frontend.keymap_picker_show_all;

    let scope = state
        .frontend
        .scope_stack
        .parent()
        .map(std::string::ToString::to_string)
        .unwrap_or_default();

    let filtered: Vec<_> = if state.frontend.keymap_picker_show_all {
        state.frontend.all_keymap_entries.clone()
    } else {
        state
            .frontend
            .all_keymap_entries
            .iter()
            .filter(|e| e.scope == scope)
            .cloned()
            .collect()
    };

    state.frontend.keymap_picker.set_items(filtered);
    IntentResult::empty()
}

// --- Private confirm handlers ---

/// Confirms the selected provider and dispatches a switch command.
fn confirm_provider(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.provider.provider_picker.selected_item() else {
        return IntentResult::empty();
    };
    if !entry.is_available {
        return IntentResult::empty();
    }
    let provider_id = entry.provider_id.clone();
    let session_id = state.session.active_session.clone();

    state.frontend.scope_stack.pop();
    IntentResult::with_commands(vec![
        Command::ProviderSwitch {
            payload: ProviderSwitch {
                session_id,
                provider_id: provider_id.clone(),
            },
        },
        Command::UpdatePreferences {
            payload: UpdatePreferences {
                updates: vec![PreferenceUpdate::SetLastModel(Some(provider_id))],
            },
        },
    ])
}

/// Confirms the selected strategy and dispatches a switch command.
fn confirm_strategy(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.context_strategy_picker.selected_item() else {
        return IntentResult::empty();
    };
    let strategy_id = entry.strategy_id.clone();
    let session_id = state.session.active_session.clone();

    state.frontend.scope_stack.pop();
    IntentResult::with_commands(vec![
        Command::SwitchPromptStrategy {
            payload: SwitchPromptStrategy {
                session_id,
                strategy_id: strategy_id.clone(),
            },
        },
        Command::UpdatePreferences {
            payload: UpdatePreferences {
                updates: vec![PreferenceUpdate::SetLastStrategy(Some(
                    strategy_id.as_str().to_owned(),
                ))],
            },
        },
    ])
}

/// Confirms a keymap selection. Returns the selected intent for the caller
/// to re-dispatch, rather than dispatching it directly (avoids circular dep).
fn confirm_keymap(state: &mut AppState) -> (IntentResult, Option<Intent>) {
    let Some(entry) = state.frontend.keymap_picker.selected_item() else {
        return (IntentResult::empty(), None);
    };
    let intent = entry.command.clone();

    state.frontend.scope_stack.pop();
    (IntentResult::empty(), Some(intent))
}

/// Confirms the selected persona and sets it as active.
fn confirm_persona(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.persona_picker.selected_item() else {
        return IntentResult::empty();
    };
    let persona_name = entry.name.clone();

    // Find the matching persona and set it as active.
    let persona = state
        .context
        .personas
        .iter()
        .find(|p| p.name == persona_name)
        .cloned();
    if let Some(p) = persona {
        state.context.active_persona = Some(p);
    }

    state.frontend.scope_stack.pop();
    IntentResult::empty()
}

/// Confirms the selected session and dispatches a switch command.
fn confirm_session(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.session_picker.selected_item() else {
        return IntentResult::empty();
    };
    let session_id = entry.session_id.clone();
    let byte_offset = entry.byte_offset;

    state.session.session_loading = true;
    state.session.session_load_started_at = Some(std::time::Instant::now());
    state.frontend.scope_stack.pop();

    IntentResult::with_commands(vec![Command::SessionLoadRequested {
        payload: SessionLoadRequested {
            session_id,
            byte_offset,
        },
    }])
}

#[cfg(test)]
mod tests {
    use crate::common::app_state::{AppState, FocusScope};
    use crate::protocol::{Command, Intent, PickerKind, SessionId};
    use crate::protocol::{KeymapEntry, PickerEntry, SessionEntry, StrategyEntry};

    use super::*;

    // --- Open picker ---

    #[rstest::rstest]
    fn open_picker_provider_sets_kind_and_mode() {
        // Given a default state.
        let mut state = AppState::default();

        // When opening a Provider picker.
        let result = handle_open_picker(&mut state, PickerKind::Provider);

        // Then scope_stack has a Picker(Provider) on top.
        assert_eq!(
            state.frontend.scope_stack.picker_kind().copied(),
            Some(PickerKind::Provider)
        );
        // And a LoadProviderPickerEntries command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::LoadProviderPickerEntries { .. }))
        );
    }

    #[rstest::rstest]
    fn open_picker_keymap_resets_show_all() {
        // Given a state with show_all=true.
        let mut state = AppState::default();
        state.frontend.keymap_picker_show_all = true;

        // When opening a Keymap picker.
        let result = handle_open_picker(&mut state, PickerKind::Keymap);

        // Then show_all is false.
        assert!(!state.frontend.keymap_picker_show_all);
        assert_eq!(
            state.frontend.scope_stack.picker_kind().copied(),
            Some(PickerKind::Keymap)
        );
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn open_picker_noop_when_already_in_picker() {
        // Given a state already in picker mode.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Session,
        });

        // When opening a Provider picker.
        let result = handle_open_picker(&mut state, PickerKind::Provider);

        // Then nothing changed.
        assert_eq!(
            state.frontend.scope_stack.picker_kind().copied(),
            Some(PickerKind::Session)
        );
        assert!(result.commands.is_empty());
    }

    // --- Insert char / backspace ---

    #[rstest::rstest]
    fn picker_insert_char_updates_filter() {
        // Given a state with active provider picker.
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
            is_alias: false,
            alias_target: None,
            is_available: true,
            is_remote: false,
            is_active: false,
        }]);

        // When inserting 't'.
        let result = handle_insert_char(&mut state, 't');

        // Then the filter contains "t".
        assert_eq!(state.provider.provider_picker.filter(), "t");
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn picker_backspace_removes_from_filter() {
        // Given a state with active provider picker and "te" in filter.
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
            is_alias: false,
            alias_target: None,
            is_available: true,
            is_remote: false,
            is_active: false,
        }]);
        state.provider.provider_picker.insert_char('t');
        state.provider.provider_picker.insert_char('e');

        // When handling backspace.
        let result = handle_backspace(&mut state);

        // Then the filter is "t".
        assert_eq!(state.provider.provider_picker.filter(), "t");
        assert!(result.commands.is_empty());
    }

    // --- Confirm ---

    #[rstest::rstest]
    fn picker_confirm_provider_returns_provider_switch() {
        // Given a state with active provider picker and a selected entry.
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
            is_alias: false,
            alias_target: None,
            is_available: true,
            is_remote: false,
            is_active: false,
        }]);

        // When confirming picker.
        let (result, maybe_intent) = handle_picker_confirm(&mut state);

        // Then a ProviderSwitch command is returned and picker is closed.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::ProviderSwitch { .. }))
        );
        assert!(!state.frontend.scope_stack.is_picker());
        assert!(maybe_intent.is_none());
    }

    #[rstest::rstest]
    fn picker_confirm_session_returns_session_load_command() {
        // Given a state with active session picker and a selected entry.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Session,
        });
        state.frontend.session_picker.set_items(vec![SessionEntry {
            session_id: SessionId::new(),
            title: "Test".to_owned(),
            updated_at: jiff::Timestamp::now(),
            byte_offset: 0,
        }]);

        // When confirming picker.
        let (result, maybe_intent) = handle_picker_confirm(&mut state);

        // Then session_loading is true.
        assert!(state.session.session_loading);
        // And session_load_started_at is set.
        assert!(state.session.session_load_started_at.is_some());
        // And a SessionLoadRequested command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::SessionLoadRequested { .. }))
        );
        // And picker is closed.
        assert!(!state.frontend.scope_stack.is_picker());
        assert!(maybe_intent.is_none());
    }

    #[rstest::rstest]
    fn picker_confirm_keymap_returns_intent_for_redispatch() {
        // Given a state with active keymap picker.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Keymap,
        });
        state.frontend.keymap_picker.set_items(vec![KeymapEntry {
            key_sequence: "q".to_owned(),
            description: "quit".to_owned(),
            scope: "Normal".to_owned(),
            category: "General".to_owned(),
            command: Intent::Quit,
            search_text: "q quit".to_owned(),
        }]);

        // When confirming picker.
        let (result, maybe_intent) = handle_picker_confirm(&mut state);

        // Then picker is closed and the intent is returned for redispatch.
        assert!(!state.frontend.scope_stack.is_picker());
        assert!(result.commands.is_empty());
        assert!(matches!(maybe_intent, Some(Intent::Quit)));
    }

    #[rstest::rstest]
    fn picker_confirm_noop_with_no_active_picker() {
        // Given a state with no active picker.
        let mut state = AppState::default();

        // When confirming picker.
        let (result, maybe_intent) = handle_picker_confirm(&mut state);

        // Then no commands and no intent.
        assert!(result.commands.is_empty());
        assert!(maybe_intent.is_none());
    }

    #[rstest::rstest]
    fn picker_confirm_strategy_updates_default() {
        // Given a state with active context strategy picker and manual entries.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::ContextAssembly,
        });
        state.frontend.context_strategy_picker.set_items(vec![
            StrategyEntry {
                strategy_id: crate::protocol::PromptStrategyId::passthrough(),
                name: "Passthrough".to_owned(),
                description: "No processing".to_owned(),
                is_active: false,
            },
            StrategyEntry {
                strategy_id: crate::protocol::PromptStrategyId::sliding_window(),
                name: "Sliding Window".to_owned(),
                description: "Sliding window".to_owned(),
                is_active: false,
            },
        ]);
        // Navigate to second entry.
        state.frontend.context_strategy_picker.move_down(100);

        // When confirming picker.
        let (result, maybe_intent) = handle_picker_confirm(&mut state);

        // Then SwitchPromptStrategy command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::SwitchPromptStrategy { .. }))
        );
        // And picker is closed.
        assert!(!state.frontend.scope_stack.is_picker());
        assert!(maybe_intent.is_none());
    }

    // --- Move up / down ---

    #[rstest::rstest]
    fn picker_move_up_decrements_selection() {
        // Given a state with active provider picker at index 1.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        state.provider.provider_picker.set_items(vec![
            PickerEntry {
                provider_id: "a".to_owned(),
                name: "a".to_owned(),
                provider_name: "a".to_owned(),
                backend: "a".to_owned(),
                model: "a".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
            },
            PickerEntry {
                provider_id: "b".to_owned(),
                name: "b".to_owned(),
                provider_name: "b".to_owned(),
                backend: "b".to_owned(),
                model: "b".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
            },
        ]);
        state.provider.provider_picker.move_down(100);
        assert_eq!(state.provider.provider_picker.selection(), 1);

        // When handling move up.
        let result = handle_move_up(&mut state);

        // Then selection is 0.
        assert_eq!(state.provider.provider_picker.selection(), 0);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn picker_move_down_increments_selection() {
        // Given a state with active provider picker at index 0.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        state.provider.provider_picker.set_items(vec![
            PickerEntry {
                provider_id: "a".to_owned(),
                name: "a".to_owned(),
                provider_name: "a".to_owned(),
                backend: "a".to_owned(),
                model: "a".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
            },
            PickerEntry {
                provider_id: "b".to_owned(),
                name: "b".to_owned(),
                provider_name: "b".to_owned(),
                backend: "b".to_owned(),
                model: "b".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
            },
        ]);

        // When handling move down.
        let result = handle_move_down(&mut state);

        // Then selection is 1.
        assert_eq!(state.provider.provider_picker.selection(), 1);
        assert!(result.commands.is_empty());
    }

    // --- Cursor movement ---

    #[rstest::rstest]
    fn picker_move_cursor_left_moves_cursor() {
        // Given a state with active provider picker with "ab" in filter.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        state.provider.provider_picker.insert_char('a');
        state.provider.provider_picker.insert_char('b');

        // When handling cursor left.
        let result = handle_move_cursor_left(&mut state);

        // Then cursor moved.
        assert_eq!(state.provider.provider_picker.cursor_pos(), 1);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn picker_move_cursor_right_moves_cursor() {
        // Given a state with cursor at start of filter.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        state.provider.provider_picker.insert_char('a');
        state.provider.provider_picker.insert_char('b');
        state.provider.provider_picker.move_cursor_left();
        state.provider.provider_picker.move_cursor_left();

        // When handling cursor right.
        let result = handle_move_cursor_right(&mut state);

        // Then cursor moved.
        assert_eq!(state.provider.provider_picker.cursor_pos(), 1);
        assert!(result.commands.is_empty());
    }

    // --- Toggle scope filter ---

    #[rstest::rstest]
    fn toggle_keymap_scope_filter_toggles_flag() {
        // Given a state with keymap entries.
        let mut state = AppState::default();
        state.frontend.all_keymap_entries = vec![KeymapEntry {
            key_sequence: "q".to_owned(),
            description: "quit".to_owned(),
            scope: "Normal".to_owned(),
            category: "General".to_owned(),
            command: Intent::Quit,
            search_text: "q quit".to_owned(),
        }];
        state.frontend.scope_stack.push(FocusScope::Input);
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Keymap,
        });

        // When handling toggle keymap scope filter.
        let result = handle_toggle_keymap_scope_filter(&mut state);

        // Then show_all is toggled to true.
        assert!(state.frontend.keymap_picker_show_all);
        assert!(result.commands.is_empty());
    }
}
