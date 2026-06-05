#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use crate::common::app_state::AppState;
use crate::feat::chat_input::{AutocompleteMatch, AutocompleteTrigger};
use crate::feat::session::phase_machine::PhaseKind;
use crate::protocol::{ChatEntry, Command};

#[rstest::rstest]
fn insert_char_appends_to_buffer() {
    // Given a default AppState.
    let mut state = AppState::default();

    // When handling InsertChar('x').
    let _ = crate::feat::chat_input::intent::handle_insert_char('x', &mut state);

    // Then the character is in the input buffer.
    assert_eq!(state.active_chat_input().text(), "x");
}

#[rstest::rstest]
fn insert_char_emits_no_commands() {
    // Given a default AppState.
    let mut state = AppState::default();

    // When handling InsertChar('x').
    let result = crate::feat::chat_input::intent::handle_insert_char('x', &mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn delete_grapheme_removes_last_char() {
    // Given a state with "ab" in the input buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");

    // When handling DeleteGrapheme.
    let _ = crate::feat::chat_input::intent::handle_delete_grapheme(&mut state);

    // Then the buffer is "a".
    assert_eq!(state.active_chat_input().text(), "a");
}

#[rstest::rstest]
fn delete_grapheme_emits_no_commands() {
    // Given a state with "ab" in the input buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");

    // When handling DeleteGrapheme.
    let result = crate::feat::chat_input::intent::handle_delete_grapheme(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn delete_grapheme_forward_removes_next_char() {
    // Given a state with "ab" and cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling DeleteGraphemeForward.
    let _ = crate::feat::chat_input::intent::handle_delete_grapheme_forward(&mut state);

    // Then the buffer is "b".
    assert_eq!(state.active_chat_input().text(), "b");
}

#[rstest::rstest]
fn delete_grapheme_forward_emits_no_commands() {
    // Given a state with "ab" and cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling DeleteGraphemeForward.
    let result = crate::feat::chat_input::intent::handle_delete_grapheme_forward(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn submit_message_returns_enqueue_command() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");

    // When handling SubmitMessage.
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then a MarkSessionInteracted and an EnqueueUserMessage command are returned.
    assert_eq!(result.commands.len(), 2);
    assert!(matches!(
        &result.commands[0],
        Command::MarkSessionInteracted(..)
    ));
    assert!(matches!(
        &result.commands[1],
        Command::EnqueueUserMessage(..)
    ));
}

#[rstest::rstest]
fn submit_message_clears_input_buffer() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");

    // When handling SubmitMessage.
    let _result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then the input buffer is reset.
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn submit_message_noop_with_empty_buffer() {
    // Given a state with an empty buffer.
    let mut state = AppState::default();

    // When handling SubmitMessage.
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then no commands are returned.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn submit_message_completes_and_submits_when_hash_autocomplete_active() {
    // Given a state with text and hash autocomplete active.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("#cod");
    let matches = vec![AutocompleteMatch {
        name: "code-review".to_owned(),
        description: "Perform code review".to_owned(),
    }];
    state
        .active_chat_input_mut()
        .activate_autocomplete(0, AutocompleteTrigger::Hash, matches);

    // When handling SubmitMessage.
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then the autocomplete is completed and the message is submitted.
    assert!(
        result
            .commands
            .iter()
            .any(|c| matches!(c, Command::EnqueueUserMessage(..))),
        "Enter should complete autocomplete and submit the message"
    );
}

#[rstest::rstest]
fn submit_message_with_hash_autocomplete_clears_buffer() {
    // Given a state with text and hash autocomplete active.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("#cod");
    let matches = vec![AutocompleteMatch {
        name: "code-review".to_owned(),
        description: "Perform code review".to_owned(),
    }];
    state
        .active_chat_input_mut()
        .activate_autocomplete(0, AutocompleteTrigger::Hash, matches);

    // When handling SubmitMessage.
    let _result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then the input buffer is cleared.
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn autocomplete_confirm_no_op_when_no_autocomplete() {
    // Given a state with no autocomplete active.
    let mut state = AppState::default();

    // When handling AutocompleteConfirm.
    let result = crate::feat::chat_input::intent::handle_autocomplete_confirm(&mut state);

    // Then nothing changes and no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_left_moves_cursor() {
    // Given a state with "ab" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");
    assert_eq!(state.active_chat_input().cursor_pos(), 2);

    // When handling MoveCursorLeft.
    let _ = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then the cursor has moved.
    assert_eq!(state.active_chat_input().cursor_pos(), 1);
}

#[rstest::rstest]
fn move_cursor_left_emits_no_commands() {
    // Given a state with "ab" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");
    assert_eq!(state.active_chat_input().cursor_pos(), 2);

    // When handling MoveCursorLeft.
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_right_moves_cursor() {
    // Given a state with cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling MoveCursorRight.
    let _ = crate::feat::chat_input::intent::handle_move_cursor_right(&mut state);

    // Then the cursor has moved.
    assert_eq!(state.active_chat_input().cursor_pos(), 1);
}

#[rstest::rstest]
fn move_cursor_right_emits_no_commands() {
    // Given a state with cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("ab");
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling MoveCursorRight.
    let result = crate::feat::chat_input::intent::handle_move_cursor_right(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_to_start_moves_cursor() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");

    // When handling MoveCursorToStart.
    let _ = crate::feat::chat_input::intent::handle_move_cursor_to_start(&mut state);

    // Then cursor is at position 0.
    assert_eq!(state.active_chat_input().cursor_pos(), 0);
}

#[rstest::rstest]
fn move_cursor_to_start_emits_no_commands() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");

    // When handling MoveCursorToStart.
    let result = crate::feat::chat_input::intent::handle_move_cursor_to_start(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_to_end_moves_cursor() {
    // Given a state with cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_grapheme_at_cursor('a');
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling MoveCursorToEnd.
    let _ = crate::feat::chat_input::intent::handle_move_cursor_to_end(&mut state);

    // Then cursor is at the end.
    assert_eq!(state.active_chat_input().cursor_pos(), 1);
}

#[rstest::rstest]
fn move_cursor_to_end_emits_no_commands() {
    // Given a state with cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_grapheme_at_cursor('a');
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling MoveCursorToEnd.
    let result = crate::feat::chat_input::intent::handle_move_cursor_to_end(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_word_left_moves_cursor() {
    // Given a state with "hello world".
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");

    // When handling MoveCursorWordLeft.
    let _ = crate::feat::chat_input::intent::handle_move_cursor_word_left(&mut state);

    // Then cursor moves.
    assert_eq!(state.active_chat_input().cursor_pos(), 0);
}

#[rstest::rstest]
fn move_cursor_word_left_emits_no_commands() {
    // Given a state with "hello world".
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");

    // When handling MoveCursorWordLeft.
    let result = crate::feat::chat_input::intent::handle_move_cursor_word_left(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_word_right_moves_cursor() {
    // Given a state with "hi" and cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling MoveCursorWordRight.
    let _ = crate::feat::chat_input::intent::handle_move_cursor_word_right(&mut state);

    // Then cursor moves to end.
    assert_eq!(state.active_chat_input().cursor_pos(), 2);
}

#[rstest::rstest]
fn move_cursor_word_right_emits_no_commands() {
    // Given a state with "hi" and cursor at start.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hi");
    state.active_chat_input_mut().move_cursor_to_start();

    // When handling MoveCursorWordRight.
    let result = crate::feat::chat_input::intent::handle_move_cursor_word_right(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn cursor_left_reactivates_autocomplete_when_re_entering_token() {
    // Given a state with "#code " in the buffer (autocomplete was dismissed by space).
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("#code ");
    // Cursor is at end (after space). Move left twice to get back into "code".
    state.active_chat_input_mut().move_cursor_left(); // cursor on space
    state.active_chat_input_mut().move_cursor_left(); // cursor on 'e'

    // When handling MoveCursorLeft.
    let _ = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then autocomplete is re-activated.
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should be re-activated when cursor re-enters token"
    );
}

#[rstest::rstest]
fn cursor_left_reactivates_autocomplete_emits_no_commands() {
    // Given a state with "#code " in the buffer (autocomplete was dismissed by space).
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("#code ");
    // Cursor is at end (after space). Move left twice to get back into "code".
    state.active_chat_input_mut().move_cursor_left(); // cursor on space
    state.active_chat_input_mut().move_cursor_left(); // cursor on 'e'

    // When handling MoveCursorLeft.
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn backspace_reactivates_autocomplete_when_re_entering_token() {
    // Given a state with "#code " and cursor at end.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("#code ");

    // When handling DeleteGrapheme (removes the space).
    let _ = crate::feat::chat_input::intent::handle_delete_grapheme(&mut state);

    // Then autocomplete is re-activated.
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should be re-activated when backspace re-enters token"
    );
}

#[rstest::rstest]
fn backspace_reactivates_autocomplete_emits_no_commands() {
    // Given a state with "#code " and cursor at end.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("#code ");

    // When handling DeleteGrapheme (removes the space).
    let result = crate::feat::chat_input::intent::handle_delete_grapheme(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn cursor_move_away_from_token_does_not_reactivate() {
    // Given a state with "hello #code" and cursor at end.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello #code");

    // When moving cursor left past the token boundary (into "hello ").
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // 'e'
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // 'd'
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // 'o'
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // 'c'
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // '#'
    // Now cursor is on '#'. Move left one more to space.
    let _ = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then autocomplete is NOT active (cursor left the token region).
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should NOT activate when cursor moves away from token"
    );
}

#[rstest::rstest]
fn cursor_move_away_emits_no_commands() {
    // Given a state with "hello #code" and cursor at end.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello #code");

    // When moving cursor left past the token boundary (into "hello ").
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // 'e'
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // 'd'
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // 'o'
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // 'c'
    crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // '#'
    // Now cursor is on '#'. Move left one more to space.
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_up_delegates_to_state() {
    // Given a default state.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_grapheme_at_cursor('a');

    // When handling MoveCursorUp.
    let result = crate::feat::chat_input::intent::handle_move_cursor_up(&mut state);

    // Then no crash and no commands.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn move_cursor_down_delegates_to_state() {
    // Given a default state.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_grapheme_at_cursor('a');

    // When handling MoveCursorDown.
    let result = crate::feat::chat_input::intent::handle_move_cursor_down(&mut state);

    // Then no crash and no commands.
    assert!(result.commands.is_empty());
}

// --- Mode transition tests ---

#[rstest::rstest]
fn enter_insert_mode_sets_mode_to_input() {
    // Given a state in Normal mode.
    let mut state = AppState::default();

    // When handling EnterInsertMode.
    let _ = crate::feat::chat_input::intent::handle_enter_insert_mode(&mut state);

    // Then scope_stack has Input on top.
    assert_eq!(
        state.frontend.scope_stack.current().mode(),
        crate::protocol::Mode::Input
    );
}

#[rstest::rstest]
fn enter_insert_mode_emits_no_commands() {
    // Given a state in Normal mode.
    let mut state = AppState::default();

    // When handling EnterInsertMode.
    let result = crate::feat::chat_input::intent::handle_enter_insert_mode(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn enter_normal_mode_returns_to_normal_scope() {
    // Given a state in Input mode.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);

    // When handling EnterNormalMode.
    let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then scope_stack is back to Normal.
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
}

#[rstest::rstest]
fn enter_normal_mode_from_input_emits_no_commands() {
    // Given a state in Input mode.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);

    // When handling EnterNormalMode.
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn enter_normal_mode_clears_picker_kind_when_leaving_picker() {
    // Given a state in Picker mode with active picker kind.
    use crate::common::app_state::FocusScope;
    use crate::protocol::PickerKind;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });

    // When handling EnterNormalMode.
    let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then scope_stack is back to Normal (no picker).
    assert!(!state.frontend.scope_stack.is_picker());
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
}

#[rstest::rstest]
fn enter_normal_mode_from_picker_emits_no_commands() {
    // Given a state in Picker mode with active picker kind.
    use crate::common::app_state::FocusScope;
    use crate::protocol::PickerKind;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });

    // When handling EnterNormalMode.
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn enter_normal_mode_from_input_with_sidebar_returns_to_normal() {
    // Given a state with sidebar and input on the scope stack.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::SidebarPersona);
    state.frontend.scope_stack.push(FocusScope::Input);

    // When handling EnterNormalMode.
    let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then scope_stack is back to Normal (not SidebarPersona).
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
    assert!(!state.frontend.scope_stack.is_sidebar());
}

#[rstest::rstest]
fn enter_normal_mode_from_sidebar_input_emits_no_commands() {
    // Given a state with sidebar and input on the scope stack.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::SidebarPersona);
    state.frontend.scope_stack.push(FocusScope::Input);

    // When handling EnterNormalMode.
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn enter_normal_mode_does_not_cancel_stream() {
    // Given a state in Input mode with active stream.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state.active_session_mut().begin_streaming();

    // When handling EnterNormalMode.
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then no CancelStream command is emitted.
    assert!(
        !result
            .commands
            .iter()
            .any(|c| matches!(c, Command::CancelStream(..)))
    );
}

#[rstest::rstest]
fn enter_normal_mode_preserves_streaming_phase() {
    // Given a state in Input mode with active stream.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state.active_session_mut().begin_streaming();

    // When handling EnterNormalMode.
    let _result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then the session is still streaming (not cancelled).
    assert!(matches!(
        state.active_session().phase(),
        PhaseKind::Streaming
    ));
}

#[rstest::rstest]
fn enter_normal_mode_does_not_drain_queue() {
    // Given a state in Input mode with active stream and queued messages.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state.active_session_mut().begin_streaming();
    state
        .active_session_mut()
        .enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
            ChatEntry::user("msg1"),
        ));
    state
        .active_session_mut()
        .enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
            ChatEntry::user("msg2"),
        ));

    // When handling EnterNormalMode.
    let _result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then the queued messages are NOT drained.
    assert_eq!(state.active_session().queue_len(), 2);
    // And the input buffer is empty.
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn enter_normal_mode_with_queue_emits_no_cancel_stream() {
    // Given a state in Input mode with active stream and queued messages.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state.active_session_mut().begin_streaming();
    state
        .active_session_mut()
        .enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
            ChatEntry::user("msg1"),
        ));
    state
        .active_session_mut()
        .enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
            ChatEntry::user("msg2"),
        ));

    // When handling EnterNormalMode.
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then no CancelStream command is emitted.
    assert!(
        !result
            .commands
            .iter()
            .any(|c| matches!(c, Command::CancelStream(..)))
    );
}

// --- NormalEscape tests ---

#[rstest::rstest]
fn normal_escape_does_not_clear_selection() {
    // Given a state with a selected entry.
    use crate::protocol::ChatEntry;

    let mut state = AppState::default();
    state.active_session_mut().push_entry(ChatEntry::user("hi"));
    // push_entry auto-selects index 0.
    assert_eq!(state.active_session().selected_entry_index(), Some(0));

    // When handling NormalEscape.
    let _ = crate::feat::chat_input::intent::handle_normal_escape(&mut state);

    // Then the selection is preserved (always-selected invariant).
    assert_eq!(state.active_session().selected_entry_index(), Some(0));
    // And cancel prompt is NOT set.
    assert!(!state.frontend.cancel_stream_prompt);
}

#[rstest::rstest]
fn normal_escape_emits_no_commands() {
    // Given a state with a selected entry.
    use crate::protocol::ChatEntry;

    let mut state = AppState::default();
    state.active_session_mut().push_entry(ChatEntry::user("hi"));
    // push_entry auto-selects index 0.
    assert_eq!(state.active_session().selected_entry_index(), Some(0));

    // When handling NormalEscape.
    let result = crate::feat::chat_input::intent::handle_normal_escape(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn normal_escape_sets_cancel_prompt_when_streaming() {
    // Given a state in Normal mode with an active stream.
    let mut state = AppState::default();
    state.active_session_mut().begin_streaming();

    // When handling NormalEscape.
    let _ = crate::feat::chat_input::intent::handle_normal_escape(&mut state);

    // Then the cancel prompt is set.
    assert!(state.frontend.cancel_stream_prompt);
}

#[rstest::rstest]
fn normal_escape_when_streaming_emits_no_commands() {
    // Given a state in Normal mode with an active stream.
    let mut state = AppState::default();
    state.active_session_mut().begin_streaming();

    // When handling NormalEscape.
    let result = crate::feat::chat_input::intent::handle_normal_escape(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn normal_escape_noop_when_idle_and_no_selection() {
    // Given a state that is idle with no selection.
    let mut state = AppState::default();

    // When handling NormalEscape.
    let _ = crate::feat::chat_input::intent::handle_normal_escape(&mut state);

    // Then cancel prompt is not set.
    assert!(!state.frontend.cancel_stream_prompt);
}

#[rstest::rstest]
fn normal_escape_when_idle_emits_no_commands() {
    // Given a state that is idle with no selection.
    let mut state = AppState::default();

    // When handling NormalEscape.
    let result = crate::feat::chat_input::intent::handle_normal_escape(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

// --- Slash autocomplete tests ---

#[rstest::rstest]
fn slash_at_position_0_triggers_autocomplete() {
    // Given a default AppState.
    let mut state = AppState::default();

    // When handling InsertChar('/').
    let _ = crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // Then autocomplete is active with Slash trigger.
    let ac = state.active_chat_input().autocomplete();
    assert!(
        ac.is_some(),
        "autocomplete should be active after '/' at position 0"
    );
    assert_eq!(ac.as_ref().unwrap().trigger(), AutocompleteTrigger::Slash);
}

#[rstest::rstest]
fn slash_at_position_0_emits_no_commands() {
    // Given a default AppState.
    let mut state = AppState::default();

    // When handling InsertChar('/').
    let result = crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn slash_does_not_trigger_with_content() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello");

    // When handling InsertChar('/').
    let _ = crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // Then autocomplete is NOT active.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should NOT trigger when buffer has content"
    );
}

#[rstest::rstest]
fn slash_with_content_emits_no_commands() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello");

    // When handling InsertChar('/').
    let result = crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn slash_autocomplete_shows_new_command() {
    // Given a state where '/' was typed at position 0.
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // Then the autocomplete popup has the /new command.
    let ac = state.active_chat_input().autocomplete().as_ref().unwrap();
    let names: Vec<&str> = ac.matches().iter().map(|m| m.name.as_str()).collect();
    assert!(
        names.contains(&"new"),
        "expected 'new' in matches, got: {names:?}"
    );
}

#[rstest::rstest]
fn slash_autocomplete_filters_on_typing() {
    // Given a state with '/n' typed.
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);
    crate::feat::chat_input::intent::handle_insert_char('n', &mut state);

    // Then the autocomplete filter matches 'n'.
    let filter = state
        .active_chat_input()
        .autocomplete_filter()
        .unwrap_or_default();
    assert_eq!(filter, "n");

    // And the new command is still in the matches.
    let ac = state.active_chat_input().autocomplete().as_ref().unwrap();
    let names: Vec<&str> = ac.matches().iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"new"), "'new' should match filter 'n'");
}

#[rstest::rstest]
fn slash_autocomplete_tab_completes_name() {
    // Given a state with slash autocomplete active.
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // Navigate to the "new" entry (default selection is the last entry).
    // Entries: compact(0), new(1), workflow(2). Default = 2. Move up once.
    let _ = crate::feat::chat_input::intent::handle_move_cursor_up(&mut state);

    // When confirming autocomplete (Tab).
    let _ = crate::feat::chat_input::intent::handle_autocomplete_confirm(&mut state);

    // Then the buffer contains "/new".
    assert_eq!(state.active_chat_input().text(), "/new");
}

#[rstest::rstest]
fn slash_autocomplete_tab_confirm_emits_no_commands() {
    // Given a state with slash autocomplete active.
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // When confirming autocomplete (Tab).
    let result = crate::feat::chat_input::intent::handle_autocomplete_confirm(&mut state);

    // Then no commands were emitted (autocomplete confirm doesn't execute).
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn slash_autocomplete_dismisses_on_space() {
    // Given a state with slash autocomplete active.
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // When pressing space.
    crate::feat::chat_input::intent::handle_insert_char(' ', &mut state);

    // Then autocomplete is dismissed.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should be dismissed on space"
    );
}

#[rstest::rstest]
fn slash_autocomplete_reactivates_on_cursor_reentry() {
    // Given a state with "/ne " (autocomplete dismissed by space).
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);
    crate::feat::chat_input::intent::handle_insert_char('n', &mut state);
    crate::feat::chat_input::intent::handle_insert_char('e', &mut state);
    crate::feat::chat_input::intent::handle_insert_char(' ', &mut state);
    assert!(state.active_chat_input().autocomplete().is_none());

    // When moving cursor left back to 'e'.
    state.active_chat_input_mut().move_cursor_left(); // on space
    state.active_chat_input_mut().move_cursor_left(); // on 'e'
    let _ = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // on 'n'

    // Then autocomplete is re-activated.
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should reactivate when cursor re-enters /token"
    );
}

#[rstest::rstest]
fn slash_autocomplete_cursor_reentry_emits_no_commands() {
    // Given a state with "/ne " (autocomplete dismissed by space).
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);
    crate::feat::chat_input::intent::handle_insert_char('n', &mut state);
    crate::feat::chat_input::intent::handle_insert_char('e', &mut state);
    crate::feat::chat_input::intent::handle_insert_char(' ', &mut state);
    assert!(state.active_chat_input().autocomplete().is_none());

    // When moving cursor left back to 'e'.
    state.active_chat_input_mut().move_cursor_left(); // on space
    state.active_chat_input_mut().move_cursor_left(); // on 'e'
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // on 'n'

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn slash_autocomplete_does_not_reactivate_after_cursor_leaves_token() {
    // Given a state with "a /ne" and cursor at end.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("a /ne");

    // When moving cursor left to the space before '/'.
    state.active_chat_input_mut().move_cursor_left(); // 'e'
    state.active_chat_input_mut().move_cursor_left(); // 'n'
    state.active_chat_input_mut().move_cursor_left(); // '/'
    let _ = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // space

    // Then autocomplete is NOT active (slash was not at position 0).
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should NOT reactivate for / not at position 0"
    );
}

#[rstest::rstest]
fn slash_autocomplete_cursor_leaves_token_emits_no_commands() {
    // Given a state with "a /ne" and cursor at end.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("a /ne");

    // When moving cursor left to the space before '/'.
    state.active_chat_input_mut().move_cursor_left(); // 'e'
    state.active_chat_input_mut().move_cursor_left(); // 'n'
    state.active_chat_input_mut().move_cursor_left(); // '/'
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // space

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

// --- Slash command execution tests ---

#[rstest::rstest]
fn submit_new_command_creates_session() {
    // Given a state with "/new" in the buffer (no autocomplete active).
    let mut state = AppState::default();
    let old_id = state.session.active_session_id().clone();
    state.active_chat_input_mut().insert_text("/new");

    // When handling SubmitMessage.
    let _result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then a new session is created.
    assert_ne!(*state.session.active_session_id(), old_id);
    // And the input buffer is cleared.
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn submit_new_command_emits_no_enqueue_command() {
    // Given a state with "/new" in the buffer (no autocomplete active).
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("/new");

    // When handling SubmitMessage.
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then no EnqueueUserMessage was emitted.
    assert!(
        !result
            .commands
            .iter()
            .any(|c| matches!(c, Command::EnqueueUserMessage(..))),
        "/new should not enqueue a chat message"
    );
}

#[rstest::rstest]
fn submit_unknown_slash_command_sends_as_chat() {
    // Given a state with "/lol" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("/lol");

    // When handling SubmitMessage.
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then the message is submitted as a normal chat message.
    // MarkSessionInteracted + EnqueueUserMessage.
    assert_eq!(result.commands.len(), 2);
    assert!(
        matches!(&result.commands[0], Command::MarkSessionInteracted(..)),
        "first command should be MarkSessionInteracted"
    );
    assert!(
        matches!(&result.commands[1], Command::EnqueueUserMessage(..)),
        "unknown /command should be sent as chat"
    );
}

#[rstest::rstest]
fn submit_unknown_slash_command_clears_buffer() {
    // Given a state with "/lol" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("/lol");

    // When handling SubmitMessage.
    let _result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then the buffer is cleared.
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn submit_compact_slash_command_pushes_system_message() {
    // Given a state with "/compact" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("/compact");

    // When handling SubmitMessage.
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then a MarkSessionInteracted and TriggerCompaction command are dispatched.
    assert_eq!(result.commands.len(), 2);
    assert!(
        matches!(&result.commands[0], Command::MarkSessionInteracted(..)),
        "first command should be MarkSessionInteracted"
    );
    assert!(
        matches!(&result.commands[1], Command::TriggerCompaction(..)),
        "second command should be TriggerCompaction"
    );
}

#[rstest::rstest]
fn submit_compact_slash_command_clears_buffer() {
    // Given a state with "/compact" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("/compact");

    // When handling SubmitMessage.
    let _result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then the buffer is cleared.
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn tab_completes_name_without_executing() {
    // Given a state with slash autocomplete active ("/" typed, popup showing entries).
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);
    let old_id = state.session.active_session_id().clone();

    // Navigate to the "new" entry (default selection is the last entry).
    let _ = crate::feat::chat_input::intent::handle_move_cursor_up(&mut state);

    // When confirming autocomplete (Tab).
    let _ = crate::feat::chat_input::intent::handle_autocomplete_confirm(&mut state);

    // Then the buffer contains "/new" (completed) but no session was created.
    assert_eq!(state.active_chat_input().text(), "/new");
    assert_eq!(
        *state.session.active_session_id(),
        old_id,
        "session should not change on Tab confirm"
    );
}

#[rstest::rstest]
fn tab_confirm_slash_emits_no_commands() {
    // Given a state with slash autocomplete active ("/" typed, popup showing "new").
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // When confirming autocomplete (Tab).
    let result = crate::feat::chat_input::intent::handle_autocomplete_confirm(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn enter_completes_and_executes_slash_command() {
    // Given a state with slash autocomplete active ("/" typed, popup showing entries).
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);
    let old_id = state.session.active_session_id().clone();

    // Navigate to the "new" entry (default selection is the last entry).
    let _ = crate::feat::chat_input::intent::handle_move_cursor_up(&mut state);

    // When pressing Enter (SubmitMessage with autocomplete active).
    let _result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then the command is completed and executed.
    assert_ne!(
        *state.session.active_session_id(),
        old_id,
        "session should change on Enter"
    );
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn enter_slash_command_emits_no_enqueue() {
    // Given a state with slash autocomplete active ("/" typed, popup showing "new").
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // When pressing Enter (SubmitMessage with autocomplete active).
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then no EnqueueUserMessage was emitted.
    assert!(
        !result
            .commands
            .iter()
            .any(|c| matches!(c, Command::EnqueueUserMessage(..))),
        "/new should not enqueue a chat message"
    );
}

// --- Paste ---

#[rstest::rstest]
fn paste_text_inserts_into_chat_input() {
    // Given a default AppState.
    let mut state = AppState::default();

    // When handling PasteText with "hello\nworld".
    let _ = crate::feat::chat_input::intent::handle_paste_text("hello\nworld", &mut state);

    // Then the buffer contains the pasted text with newlines preserved.
    assert_eq!(state.active_chat_input().text(), "hello\nworld");
}

#[rstest::rstest]
fn paste_text_emits_no_commands() {
    // Given a default AppState.
    let mut state = AppState::default();

    // When handling PasteText with "hello\nworld".
    let result = crate::feat::chat_input::intent::handle_paste_text("hello\nworld", &mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn paste_text_inserts_at_cursor_position() {
    // Given a state with "hello" and cursor at position 2.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello");
    state.active_chat_input_mut().move_cursor_to_start();
    state.active_chat_input_mut().move_cursor_right();
    state.active_chat_input_mut().move_cursor_right(); // cursor at 2

    // When handling PasteText with "XY".
    let _ = crate::feat::chat_input::intent::handle_paste_text("XY", &mut state);

    // Then text is "heXYllo" and cursor is at 4.
    assert_eq!(state.active_chat_input().text(), "heXYllo");
    assert_eq!(state.active_chat_input().cursor_pos(), 4);
}

#[rstest::rstest]
fn paste_text_at_cursor_emits_no_commands() {
    // Given a state with "hello" and cursor at position 2.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello");
    state.active_chat_input_mut().move_cursor_to_start();
    state.active_chat_input_mut().move_cursor_right();
    state.active_chat_input_mut().move_cursor_right(); // cursor at 2

    // When handling PasteText with "XY".
    let result = crate::feat::chat_input::intent::handle_paste_text("XY", &mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

// --- Hash after newline tests ---

#[rstest::rstest]
fn hash_triggers_after_newline() {
    // Given a state with "hello\n" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello\n");

    // When handling InsertChar('#').
    let _ = crate::feat::chat_input::intent::handle_insert_char('#', &mut state);

    // Then autocomplete is active with Hash trigger.
    let ac = state.active_chat_input().autocomplete();
    assert!(
        ac.is_some(),
        "autocomplete should be active after '#' on new line"
    );
    assert_eq!(ac.as_ref().unwrap().trigger(), AutocompleteTrigger::Hash);
}

#[rstest::rstest]
fn hash_after_newline_emits_no_commands() {
    // Given a state with "hello\n" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello\n");

    // When handling InsertChar('#').
    let result = crate::feat::chat_input::intent::handle_insert_char('#', &mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn hash_triggers_after_newline_at_line_start() {
    // Given a state with "\n" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("\n");

    // When handling InsertChar('#').
    let _ = crate::feat::chat_input::intent::handle_insert_char('#', &mut state);

    // Then autocomplete is active.
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should be active after '#' following newline"
    );
}

#[rstest::rstest]
fn hash_after_newline_start_emits_no_commands() {
    // Given a state with "\n" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("\n");

    // When handling InsertChar('#').
    let result = crate::feat::chat_input::intent::handle_insert_char('#', &mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn hash_does_not_trigger_after_non_boundary_char() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello");

    // When handling InsertChar('#').
    let _ = crate::feat::chat_input::intent::handle_insert_char('#', &mut state);

    // Then autocomplete is NOT active (preceded by 'o', not a boundary).
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should NOT trigger when '#' is preceded by a non-boundary char"
    );
}

#[rstest::rstest]
fn hash_after_non_boundary_emits_no_commands() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello");

    // When handling InsertChar('#').
    let result = crate::feat::chat_input::intent::handle_insert_char('#', &mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn hash_reactivates_when_cursor_enters_token_after_newline() {
    // Given a state with "hello\n#code" in buffer and no autocomplete.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello\n#code");

    // When moving cursor left into the "#code" token.
    // Cursor starts at end (position 11). Move left to 'e' (position 10).
    let _ = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then autocomplete is re-activated.
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should reactivate when cursor re-enters #token after newline"
    );
}

#[rstest::rstest]
fn hash_cursor_reentry_emits_no_commands() {
    // Given a state with "hello\n#code" in buffer and no autocomplete.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello\n#code");

    // When moving cursor left into the "#code" token.
    // Cursor starts at end (position 11). Move left to 'e' (position 10).
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

// --- Popup auto-close on cursor move past token ---

#[rstest::rstest]
fn cursor_right_past_token_deactivates_autocomplete() {
    // Given a state with "#code hello" and autocomplete active at token_start=0.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("#code hello");
    state
        .active_chat_input_mut()
        .activate_autocomplete(0, AutocompleteTrigger::Hash, vec![]);
    // Move cursor to start, then right through the token.
    // token_end = 5 (one past 'e'). Cursor at 5 == token_end, still "in" token.
    state.active_chat_input_mut().move_cursor_to_start();
    crate::feat::chat_input::intent::handle_move_cursor_right(&mut state); // cursor at 1 ('c')
    crate::feat::chat_input::intent::handle_move_cursor_right(&mut state); // cursor at 2 ('o')
    crate::feat::chat_input::intent::handle_move_cursor_right(&mut state); // cursor at 3 ('d')
    crate::feat::chat_input::intent::handle_move_cursor_right(&mut state); // cursor at 4 ('e')
    crate::feat::chat_input::intent::handle_move_cursor_right(&mut state); // cursor at 5 (space)

    // Then autocomplete is still active (cursor at token_end).
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "popup should stay open when cursor is at token_end"
    );

    // When moving cursor right one more time to position 6 (past token).
    crate::feat::chat_input::intent::handle_move_cursor_right(&mut state);

    // Then autocomplete is deactivated.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "popup should close when cursor moves past token_end"
    );
}

#[rstest::rstest]
fn cursor_right_within_token_keeps_autocomplete_active() {
    // Given a state with "#code" and autocomplete active at token_start=0.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("#code");
    state
        .active_chat_input_mut()
        .activate_autocomplete(0, AutocompleteTrigger::Hash, vec![]);
    // Move cursor to start, then right to position 2.
    state.active_chat_input_mut().move_cursor_to_start();
    crate::feat::chat_input::intent::handle_move_cursor_right(&mut state); // cursor at 1
    crate::feat::chat_input::intent::handle_move_cursor_right(&mut state); // cursor at 2

    // Then autocomplete is still active (cursor within token, position 2 < token_end=5).
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "popup should stay open when cursor is within token"
    );
}

// --- ESC dismisses autocomplete (two-level ESC) ---

#[rstest::rstest]
fn enter_normal_mode_deactivates_hash_autocomplete() {
    // Given a state in Input scope with hash autocomplete active.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state
        .active_chat_input_mut()
        .activate_autocomplete(0, AutocompleteTrigger::Hash, vec![]);

    // When handling EnterNormalMode.
    let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then autocomplete is deactivated.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "ESC should deactivate hash autocomplete"
    );
}

#[rstest::rstest]
fn enter_normal_mode_with_hash_autocomplete_stays_in_input_scope() {
    // Given a state in Input scope with hash autocomplete active.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state
        .active_chat_input_mut()
        .activate_autocomplete(0, AutocompleteTrigger::Hash, vec![]);

    // When handling EnterNormalMode.
    let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then scope is still Input (not Normal).
    assert_eq!(
        state.frontend.scope_stack.current(),
        &FocusScope::Input,
        "scope should stay in Input after dismissing autocomplete"
    );
}

#[rstest::rstest]
fn enter_normal_mode_deactivates_slash_autocomplete() {
    // Given a state in Input scope with slash autocomplete active.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state
        .active_chat_input_mut()
        .activate_autocomplete(0, AutocompleteTrigger::Slash, vec![]);

    // When handling EnterNormalMode.
    let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then autocomplete is deactivated.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "ESC should deactivate slash autocomplete"
    );
}

#[rstest::rstest]
fn enter_normal_mode_with_slash_autocomplete_stays_in_input_scope() {
    // Given a state in Input scope with slash autocomplete active.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state
        .active_chat_input_mut()
        .activate_autocomplete(0, AutocompleteTrigger::Slash, vec![]);

    // When handling EnterNormalMode.
    let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then scope is still Input (not Normal).
    assert_eq!(
        state.frontend.scope_stack.current(),
        &FocusScope::Input,
        "scope should stay in Input after dismissing slash autocomplete"
    );
}

#[rstest::rstest]
fn enter_normal_mode_without_autocomplete_switches_to_normal() {
    // Given a state in Input scope with no autocomplete.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);

    // When handling EnterNormalMode.
    let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then scope switches to Normal.
    assert_eq!(
        state.frontend.scope_stack.current(),
        &FocusScope::Normal,
        "ESC should switch to Normal when no autocomplete is active"
    );
}

#[rstest::rstest]
fn enter_normal_mode_dismissing_autocomplete_emits_no_commands() {
    // Given a state in Input scope with hash autocomplete active.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state
        .active_chat_input_mut()
        .activate_autocomplete(0, AutocompleteTrigger::Hash, vec![]);

    // When handling EnterNormalMode.
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then no commands are emitted.
    assert!(result.commands.is_empty());
}

// --- compute_matches and compute_slash_matches mutants ---

#[rstest::rstest]
fn hash_autocomplete_populates_matches_from_template_store() {
    // Given a state with a template in the store.
    use crate::common::app_state::FocusScope;
    use crate::feat::context::protocol::prompt_template::PromptTemplate;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    state.context.prompt_templates =
        crate::feat::context::prompt_template::PromptTemplateStore::from_vec(vec![
            PromptTemplate {
                name: "my_template".to_owned(),
                description: "A test template".to_owned(),
                body: "template body".to_owned(),
            },
        ]);

    // When inserting '#' at position 0.
    let _ = crate::feat::chat_input::intent::handle_insert_char('#', &mut state);

    // Then autocomplete is active with non-empty matches.
    let ac = state.active_chat_input().autocomplete();
    assert!(
        ac.is_some(),
        "autocomplete should activate on '#' with templates"
    );
    let matches = ac.as_ref().unwrap().matches();
    assert!(
        !matches.is_empty(),
        "compute_matches should return at least one match for a populated store"
    );
    assert!(
        matches.iter().any(|m| m.name == "my_template"),
        "expected 'my_template' in matches: {matches:?}"
    );
}

#[rstest::rstest]
fn slash_autocomplete_populates_matches_from_slash_commands() {
    // Given a state in Input mode.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);

    // When inserting '/' at position 0.
    let _ = crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // Then autocomplete is active with non-empty matches.
    let ac = state.active_chat_input().autocomplete();
    assert!(ac.is_some(), "autocomplete should activate on '/'");
    let matches = ac.as_ref().unwrap().matches();
    assert!(
        !matches.is_empty(),
        "compute_slash_matches should return at least one slash command"
    );
}

// ---------- CtrlClear on the chat-input scope (AC6 coverage) ----------

#[rstest::rstest]
fn ctrl_clear_input_empties_chat_input_via_handler() {
    // Given a state in Input scope with text in the buffer.
    use crate::common::app_state::FocusScope;
    use crate::feat::intent::handler::IntentHandler;
    use crate::protocol::Intent;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    let _ = crate::feat::chat_input::intent::handle_insert_char('h', &mut state);
    let _ = crate::feat::chat_input::intent::handle_insert_char('i', &mut state);
    assert!(!state.active_chat_input().is_empty());
    assert_eq!(state.active_chat_input().cursor_pos(), 2);

    // When handling CtrlClear via the IntentHandler.
    let result = IntentHandler::handle(&Intent::CtrlClear, &mut state);

    // Then the chat input is cleared and scope remains Input.
    assert!(state.active_chat_input().is_empty(), "input buffer cleared");
    assert_eq!(
        state.active_chat_input().cursor_pos(),
        0,
        "cursor reset to 0"
    );
    assert!(result.commands.is_empty(), "no commands emitted");
    assert_eq!(
        state.frontend.scope_stack.current(),
        &FocusScope::Input,
        "scope remains Input (no quit, no escape)"
    );
}

#[rstest::rstest]
fn ctrl_clear_input_empty_is_noop_via_handler() {
    // Given a state in Input scope with empty buffer.
    use crate::common::app_state::FocusScope;
    use crate::feat::intent::handler::IntentHandler;
    use crate::protocol::Intent;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);
    assert!(state.active_chat_input().is_empty());

    // When handling CtrlClear via the IntentHandler.
    let result = IntentHandler::handle(&Intent::CtrlClear, &mut state);

    // Then nothing changes: no scope change, no commands, buffer still empty.
    assert!(state.active_chat_input().is_empty(), "buffer still empty");
    assert_eq!(
        state.active_chat_input().cursor_pos(),
        0,
        "cursor still 0"
    );
    assert!(result.commands.is_empty(), "no commands emitted");
    assert_eq!(
        state.frontend.scope_stack.current(),
        &FocusScope::Input,
        "scope remains Input"
    );
}
