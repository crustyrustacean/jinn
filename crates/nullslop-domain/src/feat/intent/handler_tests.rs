// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Tests for the [`IntentHandler`] — one test per Intent variant.

use crate::KeymapEntry;
use crate::{AppState, FocusScope, PickerKind};

use crate::Intent;
use crate::IntentHandler;
use crate::protocol::PickerEntry;

fn handle(intent: &Intent, state: &mut AppState) -> super::IntentResult {
    IntentHandler::handle(intent, state)
}

// ============================================================
// Picker Intent: Keymap Confirm Re-dispatch
// ============================================================

#[rstest::rstest]
fn picker_confirm_keymap_sets_mode_and_signal() {
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

    // When handling PickerConfirm.
    let result = handle(&Intent::PickerConfirm, &mut state);

    // Then the intent was executed (should_quit is set).
    assert!(!state.frontend.scope_stack.is_picker());
    assert!(state.frontend.should_quit);
    assert!(result.commands.is_empty());
}

// ============================================================
// TUI Signals: cleared at start of each handle() call
// ============================================================

#[rstest::rstest]
fn tui_signals_are_cleared_at_start_of_handle() {
    // Given a state with stale signals from a previous call.
    let mut state = AppState::default();
    state.frontend.tui_signals.toggle_whichkey = true;
    state.frontend.tui_signals.edit_requested = true;

    // When handling any intent that doesn't set signals.
    let result = handle(&Intent::Quit, &mut state);

    // Then the previous signals are cleared (only should_quit is set).
    assert!(!state.frontend.tui_signals.toggle_whichkey);
    assert!(!state.frontend.tui_signals.edit_requested);
    assert!(state.frontend.should_quit);
    assert!(result.commands.is_empty());
}

// ============================================================
// ScopeStack integration: scope transitions via IntentHandler
// ============================================================

#[rstest::rstest]
fn sidebar_picker_confirm_returns_to_sidebar() {
    // Given a state with sidebar focus.
    let mut state = AppState::default();
    handle(&Intent::SidebarFocus, &mut state);
    assert!(state.frontend.scope_stack.is_sidebar());

    // When opening a provider picker.
    handle(
        &Intent::OpenPicker {
            kind: PickerKind::Provider,
        },
        &mut state,
    );
    assert!(state.frontend.scope_stack.is_picker());

    // When confirming the picker (need a selected item).
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
    handle(&Intent::PickerConfirm, &mut state);

    // Then the stack is back at Sidebar.
    assert!(state.frontend.scope_stack.is_sidebar());
}

#[rstest::rstest]
fn input_picker_confirm_returns_to_input() {
    // Given a state in input mode.
    let mut state = AppState::default();
    handle(&Intent::EnterInsertMode, &mut state);
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Input);

    // When opening and confirming a provider picker.
    handle(
        &Intent::OpenPicker {
            kind: PickerKind::Provider,
        },
        &mut state,
    );
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
    handle(&Intent::PickerConfirm, &mut state);

    // Then the current scope mode is Input.
    assert_eq!(
        state.frontend.scope_stack.current().mode(),
        crate::protocol::Mode::Input
    );
}

#[rstest::rstest]
fn escape_pops_one_level_from_picker() {
    // Given a state with a provider picker open.
    let mut state = AppState::default();
    handle(
        &Intent::OpenPicker {
            kind: PickerKind::Provider,
        },
        &mut state,
    );
    assert!(state.frontend.scope_stack.is_picker());

    // When entering normal mode (escape).
    handle(&Intent::EnterNormalMode, &mut state);

    // Then the stack is back to Normal.
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
}

#[rstest::rstest]
fn pop_on_base_is_noop() {
    // Given a default state (Normal base only).
    let mut state = AppState::default();
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);

    // When entering normal mode (escape) from base.
    handle(&Intent::EnterNormalMode, &mut state);

    // Then the stack still has Normal as base.
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
    assert_eq!(state.frontend.scope_stack.len(), 1);
}
