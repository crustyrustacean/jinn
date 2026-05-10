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
use nullslop_protocol::{ChatEntry, Command, Mode, PickerKind};

use super::IntentHandler;
use crate::Intent;

fn handle(intent: &Intent, state: &mut AppState) -> super::IntentResult {
    IntentHandler::handle(intent, state)
}

// ============================================================
// Mode & App Intents
// ============================================================

#[rstest::rstest]
fn quit_sets_should_quit() {
    // Given a default state.
    let mut state = AppState::default();

    // When handling Quit.
    let result = handle(&Intent::Quit, &mut state);

    // Then should_quit is true.
    assert!(state.frontend.should_quit);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn interrupt_resets_non_empty_buffer() {
    // Given a state with text in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_grapheme_at_cursor('h');

    // When handling Interrupt.
    let result = handle(&Intent::Interrupt, &mut state);

    // Then the buffer is cleared.
    assert!(state.active_chat_input().is_empty());
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn interrupt_cancels_stream_when_buffer_empty() {
    // Given a state with empty buffer and active stream.
    let mut state = AppState::default();
    state.active_session_mut().begin_streaming();

    // When handling Interrupt.
    let result = handle(&Intent::Interrupt, &mut state);

    // Then a CancelStream command is returned.
    assert_eq!(result.commands.len(), 1);
    assert!(matches!(&result.commands[0], Command::CancelStream { .. }));
    // And the session is idle (streaming was cancelled).
    assert!(state.active_session().is_idle());
}

#[rstest::rstest]
fn interrupt_noop_when_idle_and_empty() {
    // Given a state with empty buffer and idle session.
    let mut state = AppState::default();

    // When handling Interrupt.
    let result = handle(&Intent::Interrupt, &mut state);

    // Then no commands.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn interrupt_drains_queued_messages_to_input_buffer() {
    // Given a streaming session with queued messages and empty input buffer.
    let mut state = AppState::default();
    state.active_session_mut().begin_streaming();
    state.active_session_mut().enqueue_message("queued1".into());
    state.active_session_mut().enqueue_message("queued2".into());

    // When handling Interrupt.
    let result = handle(&Intent::Interrupt, &mut state);

    // Then the queued messages are drained to the input buffer.
    assert_eq!(state.active_chat_input().text(), "queued1\nqueued2");
    // And the session is idle.
    assert!(state.active_session().is_idle());
    // And a CancelStream command is returned.
    assert!(
        result
            .commands
            .iter()
            .any(|c| matches!(c, Command::CancelStream { .. }))
    );
}

#[rstest::rstest]
fn set_mode_changes_mode() {
    // Given a state in Normal mode.
    let mut state = AppState::default();
    assert_eq!(state.frontend.mode, Mode::Normal);

    // When handling SetMode(Input).
    let result = handle(&Intent::SetMode { mode: Mode::Input }, &mut state);

    // Then mode is Input.
    assert_eq!(state.frontend.mode, Mode::Input);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn set_mode_clears_picker_kind_when_leaving_picker() {
    // Given a state in Picker mode with active picker kind.
    let mut state = AppState {
        frontend: FrontendState {
            mode: Mode::Picker,
            ..FrontendState::default()
        },
        ..Default::default()
    };
    state.frontend.active_picker_kind = Some(PickerKind::Provider);

    // When handling SetMode(Normal).
    let result = handle(&Intent::SetMode { mode: Mode::Normal }, &mut state);

    // Then active_picker_kind is cleared.
    assert_eq!(state.frontend.active_picker_kind, None);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn toggle_whichkey_sets_tui_signal() {
    // Given a default state.
    let mut state = AppState::default();

    // When handling ToggleWhichkey.
    let result = handle(&Intent::ToggleWhichkey, &mut state);

    // Then the toggle_whichkey signal is set.
    assert!(state.frontend.tui_signals.toggle_whichkey);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn normal_escape_clears_selection() {
    // Given a state with a selected entry.
    let mut state = AppState::default();
    state.active_session_mut().push_entry(ChatEntry::user("hi"));
    state.active_session_mut().select_next_entry();

    // When handling NormalEscape.
    let result = handle(&Intent::NormalEscape, &mut state);

    // Then the selection is cleared.
    assert!(state.active_session().selected_entry_index().is_none());
    // And pinned_pane_close signal is set.
    assert!(state.frontend.tui_signals.pinned_pane_close);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn normal_escape_sets_close_signal_even_without_selection() {
    // Given a state with no selection.
    let mut state = AppState::default();

    // When handling NormalEscape.
    let result = handle(&Intent::NormalEscape, &mut state);

    // Then no commands.
    assert!(result.commands.is_empty());
    // But pinned_pane_close signal is still set.
    assert!(state.frontend.tui_signals.pinned_pane_close);
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

// ============================================================
// SetMode: CancelStream when leaving Input during streaming
// ============================================================

#[rstest::rstest]
fn set_mode_input_to_normal_during_streaming_cancels_stream() {
    // Given a state in Input mode with active stream.
    let mut state = AppState {
        frontend: FrontendState {
            mode: Mode::Input,
            ..FrontendState::default()
        },
        ..Default::default()
    };
    state.active_session_mut().begin_streaming();

    // When handling SetMode(Normal).
    let result = handle(&Intent::SetMode { mode: Mode::Normal }, &mut state);

    // Then a CancelStream command is returned.
    assert!(
        result
            .commands
            .iter()
            .any(|c| matches!(c, Command::CancelStream { .. }))
    );
    // And the session is idle (streaming was cancelled).
    assert!(state.active_session().is_idle());
}

#[rstest::rstest]
fn set_mode_input_to_normal_during_streaming_drains_queue() {
    // Given a state in Input mode with active stream and queued messages.
    let mut state = AppState {
        frontend: FrontendState {
            mode: Mode::Input,
            ..FrontendState::default()
        },
        ..Default::default()
    };
    state.active_session_mut().begin_streaming();
    state.active_session_mut().enqueue_message("msg1".into());
    state.active_session_mut().enqueue_message("msg2".into());

    // When handling SetMode(Normal).
    let result = handle(&Intent::SetMode { mode: Mode::Normal }, &mut state);

    // Then the queued messages are drained to the input buffer.
    assert_eq!(state.active_chat_input().text(), "msg1\nmsg2");
    // And the session is idle.
    assert!(state.active_session().is_idle());
    // And a CancelStream command is returned.
    assert!(
        result
            .commands
            .iter()
            .any(|c| matches!(c, Command::CancelStream { .. }))
    );
}
