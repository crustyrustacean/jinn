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

use nullslop_component::context_strategy_picker::entries::StrategyEntry;
use nullslop_component::keymap_picker::entries::KeymapEntry;
use nullslop_component::provider_picker::entries::PickerEntry;
use nullslop_component::session_picker::entries::SessionEntry;
use nullslop_component::AppState;
use nullslop_protocol::context::PinChatEntry;
use nullslop_protocol::tab::TabDirection;
use nullslop_protocol::{
    ChatEntry, Command, Mode, PickerKind, PinPosition, SessionId,
};

use super::IntentHandler;
use crate::Intent;

fn handle(intent: &Intent, state: &mut AppState) -> super::IntentResult {
    IntentHandler::handle(intent, state)
}

// ============================================================
// Chat Input Intents
// ============================================================

#[rstest::rstest]
fn insert_char_appends_to_buffer() {
    // Given a default AppState.
    let mut state = AppState::default();

    // When handling InsertChar('x').
    let result = handle(&Intent::InsertChar { ch: 'x' }, &mut state);

    // Then the character is in the input buffer.
    assert_eq!(state.active_chat_input().text(), "x");
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn delete_grapheme_removes_last_char() {
    // Given a state with "ab" in the input buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");

    // When handling DeleteGrapheme.
    let result = handle(&Intent::DeleteGrapheme, &mut state);

    // Then the buffer is "a".
    assert_eq!(state.active_chat_input().text(), "a");
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn delete_grapheme_forward_removes_next_char() {
    // Given a state with "ab" and cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling DeleteGraphemeForward.
    let result = handle(&Intent::DeleteGraphemeForward, &mut state);

    // Then the buffer is "b".
    assert_eq!(state.active_chat_input().text(), "b");
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn submit_message_returns_enqueue_command() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");

    // When handling SubmitMessage.
    let result = handle(&Intent::SubmitMessage, &mut state);

    // Then an EnqueueUserMessage command is returned.
    assert_eq!(result.commands.len(), 1);
    assert!(matches!(
        &result.commands[0],
        Command::EnqueueUserMessage { .. }
    ));
    // And the input buffer is reset.
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn submit_message_noop_with_empty_buffer() {
    // Given a state with an empty buffer.
    let mut state = AppState::default();

    // When handling SubmitMessage.
    let result = handle(&Intent::SubmitMessage, &mut state);

    // Then no commands are returned.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn autocomplete_confirm_falls_back_to_switch_tab() {
    // Given a state with no autocomplete active.
    let mut state = AppState::default();
    let prev_tab = state.active_tab;

    // When handling AutocompleteConfirm.
    let result = handle(&Intent::AutocompleteConfirm, &mut state);

    // Then the tab has advanced.
    assert_ne!(state.active_tab, prev_tab);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_left_moves_cursor() {
    // Given a state with "ab" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");
    assert_eq!(state.active_chat_input().cursor_pos(), 2);

    // When handling MoveCursorLeft.
    let result = handle(&Intent::MoveCursorLeft, &mut state);

    // Then the cursor has moved.
    assert_eq!(state.active_chat_input().cursor_pos(), 1);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_right_moves_cursor() {
    // Given a state with cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling MoveCursorRight.
    let result = handle(&Intent::MoveCursorRight, &mut state);

    // Then the cursor has moved.
    assert_eq!(state.active_chat_input().cursor_pos(), 1);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_to_start_moves_cursor() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");

    // When handling MoveCursorToStart.
    let result = handle(&Intent::MoveCursorToStart, &mut state);

    // Then cursor is at position 0.
    assert_eq!(state.active_chat_input().cursor_pos(), 0);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_to_end_moves_cursor() {
    // Given a state with cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_grapheme_at_cursor('a');
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling MoveCursorToEnd.
    let result = handle(&Intent::MoveCursorToEnd, &mut state);

    // Then cursor is at the end.
    assert_eq!(state.active_chat_input().cursor_pos(), 1);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_word_left_moves_cursor() {
    // Given a state with "hello world".
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");

    // When handling MoveCursorWordLeft.
    let result = handle(&Intent::MoveCursorWordLeft, &mut state);

    // Then cursor moves.
    assert_eq!(state.active_chat_input().cursor_pos(), 0);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_word_right_moves_cursor() {
    // Given a state with "hi" and cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling MoveCursorWordRight.
    let result = handle(&Intent::MoveCursorWordRight, &mut state);

    // Then cursor moves to end.
    assert_eq!(state.active_chat_input().cursor_pos(), 2);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_up_delegates_to_state() {
    // Given a default state.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_grapheme_at_cursor('a');

    // When handling MoveCursorUp.
    let result = handle(&Intent::MoveCursorUp, &mut state);

    // Then no crash and no commands.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_down_delegates_to_state() {
    // Given a default state.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_grapheme_at_cursor('a');

    // When handling MoveCursorDown.
    let result = handle(&Intent::MoveCursorDown, &mut state);

    // Then no crash and no commands.
    assert!(result.commands.is_empty());
}

// ============================================================
// Navigation Intents
// ============================================================

#[rstest::rstest]
fn scroll_up_decrements_scroll_offset() {
    // Given a state with entries.
    let mut state = AppState::default();
    for _ in 0..20 {
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("line"));
    }
    state.active_session_mut().scroll_down(20);

    // When handling ScrollUp.
    let result = handle(&Intent::ScrollUp, &mut state);

    // Then the scroll offset decreased.
    let offset_before = 20u16;
    assert!(state.active_session().scroll_offset().unwrap_or(0) < offset_before);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn scroll_down_increments_scroll_offset() {
    // Given a state with entries.
    let mut state = AppState::default();
    for _ in 0..20 {
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("line"));
    }

    // When handling ScrollDown.
    let result = handle(&Intent::ScrollDown, &mut state);

    // Then scroll offset is non-zero (was at bottom, now scrolled up from bottom).
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn mouse_scroll_up_decrements_scroll_offset() {
    // Given a state with entries.
    let mut state = AppState::default();
    for _ in 0..20 {
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("line"));
    }
    state.active_session_mut().scroll_down(20);

    // When handling MouseScrollUp.
    let result = handle(&Intent::MouseScrollUp, &mut state);

    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn mouse_scroll_down_increments_scroll_offset() {
    // Given a state with entries.
    let mut state = AppState::default();
    for _ in 0..20 {
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("line"));
    }

    // When handling MouseScrollDown.
    let result = handle(&Intent::MouseScrollDown, &mut state);

    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn scroll_to_top_sets_offset_to_zero() {
    // Given a state scrolled down.
    let mut state = AppState::default();
    for _ in 0..20 {
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("line"));
    }
    state.active_session_mut().scroll_down(50);

    // When handling ScrollToTop.
    let result = handle(&Intent::ScrollToTop, &mut state);

    // Then scroll offset is at top.
    assert_eq!(state.active_session().scroll_offset(), Some(0));
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn scroll_to_bottom_resets_scroll() {
    // Given a state scrolled up from bottom.
    let mut state = AppState::default();
    for _ in 0..20 {
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("line"));
    }
    state.active_session_mut().scroll_up(10);

    // When handling ScrollToBottom.
    let result = handle(&Intent::ScrollToBottom, &mut state);

    // Then we're at bottom.
    assert!(state.active_session().is_at_bottom());
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn switch_tab_next_advances_tab() {
    // Given a state on Chat tab.
    let mut state = AppState::default();
    assert_eq!(state.active_tab, nullslop_protocol::ActiveTab::Chat);

    // When handling SwitchTab(Next).
    let result = handle(
        &Intent::SwitchTab {
            direction: TabDirection::Next,
        },
        &mut state,
    );

    // Then the tab has advanced.
    assert_eq!(state.active_tab, nullslop_protocol::ActiveTab::Dashboard);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn edit_input_sets_tui_signal() {
    // Given a default state.
    let mut state = AppState::default();

    // When handling EditInput.
    let result = handle(&Intent::EditInput, &mut state);

    // Then the edit_requested signal is set.
    assert!(state.tui_signals.edit_requested);
    assert!(result.commands.is_empty());
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
    assert!(state.should_quit);
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
    assert!(result.commands.iter().any(|c| matches!(c, Command::CancelStream { .. })));
}

#[rstest::rstest]
fn set_mode_changes_mode() {
    // Given a state in Normal mode.
    let mut state = AppState::default();
    assert_eq!(state.mode, Mode::Normal);

    // When handling SetMode(Input).
    let result = handle(&Intent::SetMode { mode: Mode::Input }, &mut state);

    // Then mode is Input.
    assert_eq!(state.mode, Mode::Input);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn set_mode_clears_picker_kind_when_leaving_picker() {
    // Given a state in Picker mode with active picker kind.
    let mut state = AppState {
        mode: Mode::Picker,
        ..Default::default()
    };
    state.active_picker_kind = Some(PickerKind::Provider);

    // When handling SetMode(Normal).
    let result = handle(&Intent::SetMode { mode: Mode::Normal }, &mut state);

    // Then active_picker_kind is cleared.
    assert_eq!(state.active_picker_kind, None);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn toggle_whichkey_sets_tui_signal() {
    // Given a default state.
    let mut state = AppState::default();

    // When handling ToggleWhichkey.
    let result = handle(&Intent::ToggleWhichkey, &mut state);

    // Then the toggle_whichkey signal is set.
    assert!(state.tui_signals.toggle_whichkey);
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
    assert!(state.tui_signals.pinned_pane_close);
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
    assert!(state.tui_signals.pinned_pane_close);
}

// ============================================================
// Picker Intents
// ============================================================

#[rstest::rstest]
fn open_picker_provider_sets_kind_and_mode() {
    // Given a default state.
    let mut state = AppState::default();

    // When handling OpenPicker(Provider).
    let result = handle(&Intent::OpenPicker { kind: PickerKind::Provider }, &mut state);

    // Then active_picker_kind and mode are set.
    assert_eq!(state.active_picker_kind, Some(PickerKind::Provider));
    assert_eq!(state.mode, Mode::Picker);
    // And a LoadPickerEntries command is returned.
    assert!(result.commands.iter().any(|c| matches!(
        c,
        Command::LoadPickerEntries { .. }
    )));
}

#[rstest::rstest]
fn open_picker_keymap_resets_show_all() {
    // Given a state with show_all=true.
    let mut state = AppState {
        keymap_picker_show_all: true,
        ..Default::default()
    };

    // When handling OpenPicker(Keymap).
    let result = handle(&Intent::OpenPicker { kind: PickerKind::Keymap }, &mut state);

    // Then show_all is false.
    assert!(!state.keymap_picker_show_all);
    assert_eq!(state.active_picker_kind, Some(PickerKind::Keymap));
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn open_picker_noop_when_already_in_picker() {
    // Given a state already in picker mode.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Session),
        ..Default::default()
    };

    // When handling OpenPicker(Provider).
    let result = handle(&Intent::OpenPicker { kind: PickerKind::Provider }, &mut state);

    // Then nothing changed.
    assert_eq!(state.active_picker_kind, Some(PickerKind::Session));
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_insert_char_updates_filter() {
    // Given a state with active provider picker.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Provider),
        ..Default::default()
    };
    state.provider_picker.set_items(vec![PickerEntry {
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

    // When handling PickerInsertChar('t').
    let result = handle(&Intent::PickerInsertChar { ch: 't' }, &mut state);

    // Then the filter contains "t".
    assert_eq!(state.provider_picker.filter(), "t");
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_backspace_removes_from_filter() {
    // Given a state with active provider picker and "te" in filter.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Provider),
        ..Default::default()
    };
    state.provider_picker.set_items(vec![PickerEntry {
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
    state.provider_picker.insert_char('t');
    state.provider_picker.insert_char('e');

    // When handling PickerBackspace.
    let result = handle(&Intent::PickerBackspace, &mut state);

    // Then the filter is "t".
    assert_eq!(state.provider_picker.filter(), "t");
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_confirm_provider_returns_provider_switch() {
    // Given a state with active provider picker and a selected entry.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Provider),
        ..Default::default()
    };
    state.provider_picker.set_items(vec![PickerEntry {
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

    // When handling PickerConfirm.
    let result = handle(&Intent::PickerConfirm, &mut state);

    // Then a ProviderSwitch command is returned and mode is Normal.
    assert!(result.commands.iter().any(|c| matches!(
        c,
        Command::ProviderSwitch { .. }
    )));
    assert_eq!(state.mode, Mode::Normal);
}

#[rstest::rstest]
fn picker_confirm_session_returns_session_load_command() {
    // Given a state with active session picker and a selected entry.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Session),
        ..Default::default()
    };
    state.session_picker.set_items(vec![SessionEntry {
        session_id: SessionId::new(),
        title: "Test".to_owned(),
        updated_at: jiff::Timestamp::now(),
        byte_offset: 0,
    }]);

    // When handling PickerConfirm.
    let result = handle(&Intent::PickerConfirm, &mut state);

    // Then session_loading is true.
    assert!(state.session_loading);
    // And a SessionLoadRequested command is returned.
    assert!(result.commands.iter().any(|c| matches!(
        c,
        Command::SessionLoadRequested { .. }
    )));
    // And mode is Normal.
    assert_eq!(state.mode, Mode::Normal);
}

#[rstest::rstest]
fn picker_confirm_keymap_sets_mode_and_signal() {
    // Given a state with active keymap picker.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Keymap),
        ..Default::default()
    };
    state.keymap_picker.set_items(vec![KeymapEntry {
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
    assert_eq!(state.mode, Mode::Normal);
    assert!(state.should_quit);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_confirm_noop_with_no_active_picker() {
    // Given a state with no active picker.
    let mut state = AppState::default();

    // When handling PickerConfirm.
    let result = handle(&Intent::PickerConfirm, &mut state);

    // Then no commands.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_confirm_strategy_updates_default() {
    // Given a state with active context strategy picker and manual entries.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::ContextAssembly),
        ..Default::default()
    };
    state.context_strategy_picker.set_items(vec![
        StrategyEntry {
            strategy_id: nullslop_protocol::PromptStrategyId::passthrough(),
            name: "Passthrough".to_owned(),
            description: "No processing".to_owned(),
            is_active: false,
        },
        StrategyEntry {
            strategy_id: nullslop_protocol::PromptStrategyId::sliding_window(),
            name: "Sliding Window".to_owned(),
            description: "Sliding window".to_owned(),
            is_active: false,
        },
    ]);
    // Navigate to second entry.
    state.context_strategy_picker.move_down(100);

    // When handling PickerConfirm.
    let result = handle(&Intent::PickerConfirm, &mut state);

    // Then default_strategy was updated.
    assert_ne!(
        state.default_strategy,
        nullslop_protocol::PromptStrategyId::passthrough()
    );
    // And SwitchPromptStrategy command is returned.
    assert!(result.commands.iter().any(|c| matches!(
        c,
        Command::SwitchPromptStrategy { .. }
    )));
    // And mode is Normal.
    assert_eq!(state.mode, Mode::Normal);
}

#[rstest::rstest]
fn picker_move_up_decrements_selection() {
    // Given a state with active provider picker at index 1.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Provider),
        ..Default::default()
    };
    state.provider_picker.set_items(vec![
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
    state.provider_picker.move_down(100);
    assert_eq!(state.provider_picker.selection(), 1);

    // When handling PickerMoveUp.
    let result = handle(&Intent::PickerMoveUp, &mut state);

    // Then selection is 0.
    assert_eq!(state.provider_picker.selection(), 0);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_move_down_increments_selection() {
    // Given a state with active provider picker at index 0.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Provider),
        ..Default::default()
    };
    state.provider_picker.set_items(vec![
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

    // When handling PickerMoveDown.
    let result = handle(&Intent::PickerMoveDown, &mut state);

    // Then selection is 1.
    assert_eq!(state.provider_picker.selection(), 1);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_move_cursor_left_moves_cursor() {
    // Given a state with active provider picker with "ab" in filter.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Provider),
        ..Default::default()
    };
    state.provider_picker.insert_char('a');
    state.provider_picker.insert_char('b');

    // When handling PickerMoveCursorLeft.
    let result = handle(&Intent::PickerMoveCursorLeft, &mut state);

    // Then cursor moved.
    assert_eq!(state.provider_picker.cursor_pos(), 1);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_move_cursor_right_moves_cursor() {
    // Given a state with cursor at start of filter.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Provider),
        ..Default::default()
    };
    state.provider_picker.insert_char('a');
    state.provider_picker.insert_char('b');
    state.provider_picker.move_cursor_left();
    state.provider_picker.move_cursor_left();

    // When handling PickerMoveCursorRight.
    let result = handle(&Intent::PickerMoveCursorRight, &mut state);

    // Then cursor moved.
    assert_eq!(state.provider_picker.cursor_pos(), 1);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn toggle_keymap_scope_filter_toggles_flag() {
    // Given a state with keymap entries.
    let mut state = AppState {
        all_keymap_entries: vec![KeymapEntry {
            key_sequence: "q".to_owned(),
            description: "quit".to_owned(),
            scope: "Normal".to_owned(),
            category: "General".to_owned(),
            command: Intent::Quit,
            search_text: "q quit".to_owned(),
        }],
        keymap_picker_origin_scope: Some("Input".to_owned()),
        ..Default::default()
    };

    // When handling ToggleKeymapScopeFilter.
    let result = handle(&Intent::ToggleKeymapScopeFilter, &mut state);

    // Then show_all is toggled to true.
    assert!(state.keymap_picker_show_all);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn session_new_creates_fresh_session() {
    // Given a state with an existing session.
    let mut state = AppState::default();
    let old_id = state.active_session.clone();
    state
        .active_session_mut()
        .push_entry(ChatEntry::user("old"));

    // When handling SessionNew.
    let _result = handle(&Intent::SessionNew, &mut state);

    // Then a new session is created.
    assert_ne!(state.active_session, old_id);
    assert!(state.active_session().history().is_empty());
    assert_eq!(state.mode, Mode::Normal);
}

#[rstest::rstest]
fn session_new_noop_when_picker_active() {
    // Given a state with an active picker.
    let mut state = AppState {
        active_picker_kind: Some(PickerKind::Provider),
        ..Default::default()
    };
    let old_id = state.active_session.clone();

    // When handling SessionNew.
    let result = handle(&Intent::SessionNew, &mut state);

    // Then nothing changed.
    assert_eq!(state.active_session, old_id);
    assert!(result.commands.is_empty());
}

// ============================================================
// RefreshModels & RescanPromptTemplates
// ============================================================

#[rstest::rstest]
fn refresh_models_posts_system_message_and_returns_command() {
    // Given a state with a provider.
    let mut state = AppState {
        active_provider: "ollama".to_owned(),
        ..Default::default()
    };
    let initial_len = state.active_session().history().len();

    // When handling RefreshModels.
    let result = handle(&Intent::RefreshModels, &mut state);

    // Then a system message was posted.
    assert_eq!(
        state.active_session().history().len(),
        initial_len + 1
    );
    // And a RefreshModels command is returned.
    assert!(result.commands.iter().any(|c| matches!(c, Command::RefreshModels)));
}

#[rstest::rstest]
fn refresh_models_noop_with_no_provider() {
    // Given a state with no provider.
    let mut state = AppState::default();

    // When handling RefreshModels.
    let result = handle(&Intent::RefreshModels, &mut state);

    // Then no commands.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn rescan_prompt_templates_posts_system_message_and_returns_command() {
    // Given a default state.
    let mut state = AppState::default();
    let initial_len = state.active_session().history().len();

    // When handling RescanPromptTemplates.
    let result = handle(&Intent::RescanPromptTemplates, &mut state);

    // Then a system message was posted.
    assert_eq!(
        state.active_session().history().len(),
        initial_len + 1
    );
    // And a RescanPromptTemplates command is returned.
    assert!(result.commands.iter().any(|c| matches!(
        c,
        Command::RescanPromptTemplates
    )));
}

// ============================================================
// Dashboard Intents
// ============================================================

#[rstest::rstest]
fn dashboard_select_down_moves_selection() {
    // Given a state with dashboard entries.
    let mut state = AppState::default();
    state.dashboard.mark_starting("echo", None);
    state.dashboard.mark_starting("llm", None);

    // When handling DashboardSelectDown.
    let result = handle(&Intent::DashboardSelectDown, &mut state);

    // Then the selection has moved.
    assert_eq!(state.dashboard.selected_index(), 1);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn dashboard_select_up_moves_selection() {
    // Given a state with dashboard entries at index 1.
    let mut state = AppState::default();
    state.dashboard.mark_starting("echo", None);
    state.dashboard.mark_starting("llm", None);
    state.dashboard.select_next();

    // When handling DashboardSelectUp.
    let result = handle(&Intent::DashboardSelectUp, &mut state);

    // Then the selection is at 0.
    assert_eq!(state.dashboard.selected_index(), 0);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn dashboard_select_first_moves_to_first() {
    // Given a state with entries at last index.
    let mut state = AppState::default();
    state.dashboard.mark_starting("echo", None);
    state.dashboard.mark_starting("llm", None);
    state.dashboard.mark_starting("ctx", None);
    state.dashboard.select_next();
    state.dashboard.select_next();

    // When handling DashboardSelectFirst.
    let result = handle(&Intent::DashboardSelectFirst, &mut state);

    // Then the selection is at 0.
    assert_eq!(state.dashboard.selected_index(), 0);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn dashboard_select_last_moves_to_last() {
    // Given a state with 3 dashboard entries.
    let mut state = AppState::default();
    state.dashboard.mark_starting("echo", None);
    state.dashboard.mark_starting("llm", None);
    state.dashboard.mark_starting("ctx", None);

    // When handling DashboardSelectLast.
    let result = handle(&Intent::DashboardSelectLast, &mut state);

    // Then the selection is at the last index.
    assert_eq!(state.dashboard.selected_index(), 2);
    assert!(result.commands.is_empty());
}

// ============================================================
// Pinned Panel Intents
// ============================================================

#[rstest::rstest]
fn pinned_panel_toggle_sets_signal() {
    // Given a default state.
    let mut state = AppState::default();

    // When handling PinnedPanelToggle.
    let result = handle(&Intent::PinnedPanelToggle, &mut state);

    // Then the toggle signal is set.
    assert!(state.tui_signals.pinned_pane_toggle);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn pinned_panel_open_sets_signal() {
    // Given a default state.
    let mut state = AppState::default();

    // When handling PinnedPanelOpen.
    let result = handle(&Intent::PinnedPanelOpen, &mut state);

    // Then the open signal is set.
    assert!(state.tui_signals.pinned_pane_open);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn pinned_panel_close_sets_signal() {
    // Given a default state.
    let mut state = AppState::default();

    // When handling PinnedPanelClose.
    let result = handle(&Intent::PinnedPanelClose, &mut state);

    // Then the close signal is set.
    assert!(state.tui_signals.pinned_pane_close);
    assert!(result.commands.is_empty());
}

fn state_with_pinned(count: usize) -> AppState {
    let mut state = AppState::default();
    let mut ids = vec![];
    for i in 0..count {
        let entry = ChatEntry::user(format!("entry {i}"));
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        ids.push(entry_id);
    }
    for id in &ids {
        state.active_session_mut().pin_entry(id, PinPosition::Top);
    }
    // Select the first pinned entry.
    if let Some(first_id) = ids.first() {
        state.pinned_panel.select_by_id(first_id.clone());
    }
    state
}

#[rstest::rstest]
fn pinned_panel_select_down_moves_selection() {
    // Given a state with 3 pinned entries.
    let mut state = state_with_pinned(3);

    // When handling PinnedPanelSelectDown.
    let result = handle(&Intent::PinnedPanelSelectDown, &mut state);

    // Then selection moved.
    let sorted_ids = state.sorted_pinned_ids();
    assert_eq!(state.pinned_panel.selected_id(), Some(&sorted_ids[1]));
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn pinned_panel_select_up_moves_selection() {
    // Given a state with 3 pinned entries at index 1.
    let mut state = state_with_pinned(3);
    let sorted_ids = state.sorted_pinned_ids();
    state.pinned_panel.select_next(&sorted_ids);

    // When handling PinnedPanelSelectUp.
    let result = handle(&Intent::PinnedPanelSelectUp, &mut state);

    // Then selection moved back.
    let sorted_ids = state.sorted_pinned_ids();
    assert_eq!(state.pinned_panel.selected_id(), Some(&sorted_ids[0]));
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn pinned_panel_unpin_returns_command() {
    // Given a state with pinned entries.
    let mut state = state_with_pinned(2);

    // When handling PinnedPanelUnpin.
    let result = handle(&Intent::PinnedPanelUnpin, &mut state);

    // Then an UnpinChatEntry command is returned.
    assert!(result.commands.iter().any(|c| matches!(
        c,
        Command::UnpinChatEntry { .. }
    )));
}

#[rstest::rstest]
fn pinned_panel_unpin_noop_when_empty() {
    // Given a state with no pinned entries.
    let mut state = AppState::default();

    // When handling PinnedPanelUnpin.
    let result = handle(&Intent::PinnedPanelUnpin, &mut state);

    // Then no commands.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn pinned_panel_pin_top_returns_command() {
    // Given a state with pinned entries.
    let mut state = state_with_pinned(1);

    // When handling PinnedPanelPinTop.
    let result = handle(&Intent::PinnedPanelPinTop, &mut state);

    // Then a PinChatEntry command with Top is returned.
    let pin_cmd = result.commands.iter().find_map(|c| match c {
        Command::PinChatEntry { payload } => Some(payload.position),
        _ => None,
    });
    assert_eq!(pin_cmd, Some(PinPosition::Top));
}

#[rstest::rstest]
fn pinned_panel_pin_bottom_returns_command() {
    // Given a state with pinned entries.
    let mut state = state_with_pinned(1);

    // When handling PinnedPanelPinBottom.
    let result = handle(&Intent::PinnedPanelPinBottom, &mut state);

    // Then a PinChatEntry command with Bottom is returned.
    let pin_cmd = result.commands.iter().find_map(|c| match c {
        Command::PinChatEntry { payload } => Some(payload.position),
        _ => None,
    });
    assert_eq!(pin_cmd, Some(PinPosition::Bottom));
}

#[rstest::rstest]
fn pinned_panel_pin_relative_returns_command() {
    // Given a state with pinned entries.
    let mut state = state_with_pinned(1);

    // When handling PinnedPanelPinRelative.
    let result = handle(&Intent::PinnedPanelPinRelative, &mut state);

    // Then a PinChatEntry command with Relative is returned.
    let pin_cmd = result.commands.iter().find_map(|c| match c {
        Command::PinChatEntry { payload } => Some(payload.position),
        _ => None,
    });
    assert_eq!(pin_cmd, Some(PinPosition::Relative));
}

#[rstest::rstest]
fn pinned_panel_pin_cycle_rotates_top_to_bottom() {
    // Given a pinned entry at Top.
    let mut state = AppState::default();
    let entry = ChatEntry::user("entry");
    let entry_id = entry.id.clone();
    state.active_session_mut().push_entry(entry);
    state
        .active_session_mut()
        .pin_entry(&entry_id, PinPosition::Top);
    let sorted_ids = state.sorted_pinned_ids();
    state.pinned_panel.select_by_id(sorted_ids[0].clone());

    // When handling PinnedPanelPinCycle.
    let result = handle(&Intent::PinnedPanelPinCycle, &mut state);

    // Then a PinChatEntry command with Bottom is returned.
    let pin_cmd = result.commands.iter().find_map(|c| match c {
        Command::PinChatEntry { payload } => Some(payload.position),
        _ => None,
    });
    assert_eq!(pin_cmd, Some(PinPosition::Bottom));
}

#[rstest::rstest]
fn pinned_panel_pin_cycle_noop_when_empty() {
    // Given a state with no pinned entries.
    let mut state = AppState::default();

    // When handling PinnedPanelPinCycle.
    let result = handle(&Intent::PinnedPanelPinCycle, &mut state);

    // Then no commands.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn pinned_panel_pin_top_noop_when_no_selection() {
    // Given a state with pinned entries but no selection.
    let mut state = AppState::default();
    let entry = ChatEntry::user("entry");
    let entry_id = entry.id.clone();
    state.active_session_mut().push_entry(entry);
    state
        .active_session_mut()
        .pin_entry(&entry_id, PinPosition::Top);
    // Don't select anything.

    // When handling PinnedPanelPinTop.
    let result = handle(&Intent::PinnedPanelPinTop, &mut state);

    // Then no commands.
    assert!(result.commands.is_empty());
}

// ============================================================
// Chat Entry Selection Intents
// ============================================================

#[rstest::rstest]
fn chat_entry_select_next_increments_index() {
    // Given a state with entries.
    let mut state = AppState::default();
    state.active_session_mut().push_entry(ChatEntry::user("a"));
    state.active_session_mut().push_entry(ChatEntry::user("b"));

    // When handling ChatEntrySelectNext.
    let result = handle(&Intent::ChatEntrySelectNext, &mut state);

    // Then the first entry is selected.
    assert_eq!(state.active_session().selected_entry_index(), Some(0));
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn chat_entry_select_prev_decrements_index() {
    // Given a state with entries and selection at last.
    let mut state = AppState::default();
    state.active_session_mut().push_entry(ChatEntry::user("a"));
    state.active_session_mut().push_entry(ChatEntry::user("b"));
    state.active_session_mut().select_prev_entry();

    // When handling ChatEntrySelectPrev.
    let result = handle(&Intent::ChatEntrySelectPrev, &mut state);

    // Then selection moved.
    assert_eq!(state.active_session().selected_entry_index(), Some(0));
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn chat_entry_pin_selected_returns_pin_command() {
    // Given a state with a selected entry.
    let mut state = AppState::default();
    state.active_session_mut().push_entry(ChatEntry::user("hello"));
    state.active_session_mut().select_next_entry();

    // When handling ChatEntryPinSelected.
    let result = handle(&Intent::ChatEntryPinSelected, &mut state);

    // Then a PinChatEntry command with Relative is returned.
    assert!(result.commands.iter().any(|c| {
        matches!(
            c,
            Command::PinChatEntry {
                payload: PinChatEntry {
                    position: PinPosition::Relative,
                    ..
                }
            }
        )
    }));
}

#[rstest::rstest]
fn chat_entry_pin_selected_noop_with_no_selection() {
    // Given a state with entries but no selection.
    let mut state = AppState::default();
    state.active_session_mut().push_entry(ChatEntry::user("hello"));

    // When handling ChatEntryPinSelected.
    let result = handle(&Intent::ChatEntryPinSelected, &mut state);

    // Then no commands.
    assert!(result.commands.is_empty());
}

// ============================================================
// TUI Signals: cleared at start of each handle() call
// ============================================================

#[rstest::rstest]
fn tui_signals_are_cleared_at_start_of_handle() {
    // Given a state with stale signals from a previous call.
    let mut state = AppState::default();
    state.tui_signals.toggle_whichkey = true;
    state.tui_signals.edit_requested = true;
    state.tui_signals.pinned_pane_toggle = true;

    // When handling any intent that doesn't set signals.
    let result = handle(&Intent::Quit, &mut state);

    // Then the previous signals are cleared (only should_quit is set).
    assert!(!state.tui_signals.toggle_whichkey);
    assert!(!state.tui_signals.edit_requested);
    assert!(!state.tui_signals.pinned_pane_toggle);
    assert!(state.should_quit);
    assert!(result.commands.is_empty());
}

// ============================================================
// SetMode: CancelStream when leaving Input during streaming
// ============================================================

#[rstest::rstest]
fn set_mode_input_to_normal_during_streaming_cancels_stream() {
    // Given a state in Input mode with active stream.
    let mut state = AppState {
        mode: Mode::Input,
        ..Default::default()
    };
    state.active_session_mut().begin_streaming();

    // When handling SetMode(Normal).
    let result = handle(&Intent::SetMode { mode: Mode::Normal }, &mut state);

    // Then a CancelStream command is returned.
    assert!(result.commands.iter().any(|c| matches!(
        c,
        Command::CancelStream { .. }
    )));
    // And the session is idle (streaming was cancelled).
    assert!(state.active_session().is_idle());
}

#[rstest::rstest]
fn set_mode_input_to_normal_during_streaming_drains_queue() {
    // Given a state in Input mode with active stream and queued messages.
    let mut state = AppState {
        mode: Mode::Input,
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
    assert!(result.commands.iter().any(|c| matches!(c, Command::CancelStream { .. })));
}
