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

use nullslop_component::keymap_picker::entries::KeymapEntry;
use nullslop_component::{AppState, FrontendState};
use nullslop_protocol::{Mode, PickerKind};

use super::IntentHandler;
use crate::Intent;

fn handle(intent: &Intent, state: &mut AppState) -> super::IntentResult {
    IntentHandler::handle(intent, state)
}

// ============================================================
// Picker Intent: Keymap Confirm Re-dispatch
// ============================================================

#[rstest::rstest]
fn picker_confirm_keymap_sets_mode_and_signal() {
    // Given a state with active keymap picker.
    let mut state = AppState {
        frontend: FrontendState {
            active_picker_kind: Some(PickerKind::Keymap),
            ..FrontendState::default()
        },
        ..Default::default()
    };
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

    // Then mode is Normal and the intent was executed (should_quit is set).
    assert_eq!(state.frontend.mode, Mode::Normal);
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
    state.frontend.tui_signals.pinned_pane_toggle = true;

    // When handling any intent that doesn't set signals.
    let result = handle(&Intent::Quit, &mut state);

    // Then the previous signals are cleared (only should_quit is set).
    assert!(!state.frontend.tui_signals.toggle_whichkey);
    assert!(!state.frontend.tui_signals.edit_requested);
    assert!(!state.frontend.tui_signals.pinned_pane_toggle);
    assert!(state.frontend.should_quit);
    assert!(result.commands.is_empty());
}
