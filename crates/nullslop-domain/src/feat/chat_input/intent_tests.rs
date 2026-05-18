use crate::common::app_state::AppState;
use crate::feat::chat_input::{AutocompleteMatch, AutocompleteTrigger};
use crate::protocol::{ChatEntry, Command};

#[rstest::rstest]
fn insert_char_appends_to_buffer() {
    // Given a default AppState.
    let mut state = AppState::default();

    // When handling InsertChar('x').
    let result = crate::feat::chat_input::intent::handle_insert_char('x', &mut state);

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
    let result = crate::feat::chat_input::intent::handle_delete_grapheme(&mut state);

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
    let result = crate::feat::chat_input::intent::handle_delete_grapheme_forward(&mut state);

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
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then an EnqueueUserMessage command is returned.
    assert_eq!(result.commands.len(), 1);
    assert!(matches!(
        &result.commands[0],
        Command::EnqueueUserMessage(..)
    ));
    // And the input buffer is reset.
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
    // And the input buffer is cleared.
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
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

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
    let result = crate::feat::chat_input::intent::handle_move_cursor_right(&mut state);

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
    let result = crate::feat::chat_input::intent::handle_move_cursor_to_start(&mut state);

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
    let result = crate::feat::chat_input::intent::handle_move_cursor_to_end(&mut state);

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
    let result = crate::feat::chat_input::intent::handle_move_cursor_word_left(&mut state);

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
    let result = crate::feat::chat_input::intent::handle_move_cursor_word_right(&mut state);

    // Then cursor moves to end.
    assert_eq!(state.active_chat_input().cursor_pos(), 2);
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
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then autocomplete is re-activated.
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should be re-activated when cursor re-enters token"
    );
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn backspace_reactivates_autocomplete_when_re_entering_token() {
    // Given a state with "#code " and cursor at end.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("#code ");

    // When handling DeleteGrapheme (removes the space).
    let result = crate::feat::chat_input::intent::handle_delete_grapheme(&mut state);

    // Then autocomplete is re-activated.
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should be re-activated when backspace re-enters token"
    );
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
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state);

    // Then autocomplete is NOT active (cursor left the token region).
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should NOT activate when cursor moves away from token"
    );
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
    let result = crate::feat::chat_input::intent::handle_enter_insert_mode(&mut state);

    // Then scope_stack has Input on top.
    assert_eq!(
        state.frontend.scope_stack.current().mode(),
        crate::protocol::Mode::Input
    );
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn enter_normal_mode_returns_to_normal_scope() {
    // Given a state in Input mode.
    use crate::common::app_state::FocusScope;

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Input);

    // When handling EnterNormalMode.
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then scope_stack is back to Normal.
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
    // And no commands are emitted.
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
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then scope_stack is back to Normal (no picker).
    assert!(!state.frontend.scope_stack.is_picker());
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
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
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then scope_stack is back to Normal (not SidebarPersona).
    assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
    assert!(!state.frontend.scope_stack.is_sidebar());
    // And no commands are emitted.
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
    // And the session is still streaming (not cancelled).
    assert!(state.active_session().is_streaming());
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
        .enqueue_message(ChatEntry::user("msg1"));
    state
        .active_session_mut()
        .enqueue_message(ChatEntry::user("msg2"));

    // When handling EnterNormalMode.
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then the queued messages are NOT drained.
    assert_eq!(state.active_session().queue_len(), 2);
    // And the input buffer is empty.
    assert!(state.active_chat_input().is_empty());
    // And no CancelStream command is emitted.
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
    let result = crate::feat::chat_input::intent::handle_normal_escape(&mut state);

    // Then the selection is preserved (always-selected invariant).
    assert_eq!(state.active_session().selected_entry_index(), Some(0));
    assert!(result.commands.is_empty());
    // And cancel prompt is NOT set.
    assert!(!state.frontend.cancel_stream_prompt);
}

#[rstest::rstest]
fn normal_escape_sets_cancel_prompt_when_streaming() {
    // Given a state in Normal mode with an active stream.
    let mut state = AppState::default();
    state.active_session_mut().begin_streaming();

    // When handling NormalEscape.
    let result = crate::feat::chat_input::intent::handle_normal_escape(&mut state);

    // Then the cancel prompt is set.
    assert!(state.frontend.cancel_stream_prompt);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn normal_escape_noop_when_idle_and_no_selection() {
    // Given a state that is idle with no selection.
    let mut state = AppState::default();

    // When handling NormalEscape.
    let result = crate::feat::chat_input::intent::handle_normal_escape(&mut state);

    // Then nothing happens.
    assert!(!state.frontend.cancel_stream_prompt);
    assert!(result.commands.is_empty());
}

// --- Slash autocomplete tests ---

#[rstest::rstest]
fn slash_at_position_0_triggers_autocomplete() {
    // Given a default AppState.
    let mut state = AppState::default();

    // When handling InsertChar('/').
    let result = crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // Then autocomplete is active with Slash trigger.
    let ac = state.active_chat_input().autocomplete();
    assert!(
        ac.is_some(),
        "autocomplete should be active after '/' at position 0"
    );
    assert_eq!(ac.as_ref().unwrap().trigger(), AutocompleteTrigger::Slash);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn slash_does_not_trigger_with_content() {
    // Given a state with "hello" in the buffer.
    let mut state = AppState::default();
    state.active_chat_input_mut().insert_text("hello");

    // When handling InsertChar('/').
    let result = crate::feat::chat_input::intent::handle_insert_char('/', &mut state);

    // Then autocomplete is NOT active.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should NOT trigger when buffer has content"
    );
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

    // When confirming autocomplete (Tab).
    let result = crate::feat::chat_input::intent::handle_autocomplete_confirm(&mut state);

    // Then the buffer contains "/new".
    assert_eq!(state.active_chat_input().text(), "/new");
    // And no commands were emitted (autocomplete confirm doesn't execute).
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
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // on 'n'

    // Then autocomplete is re-activated.
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should reactivate when cursor re-enters /token"
    );
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
    let result = crate::feat::chat_input::intent::handle_move_cursor_left(&mut state); // space

    // Then autocomplete is NOT active (slash was not at position 0).
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should NOT reactivate for / not at position 0"
    );
    assert!(result.commands.is_empty());
}

// --- Slash command execution tests ---

#[rstest::rstest]
fn submit_new_command_creates_session() {
    // Given a state with "/new" in the buffer (no autocomplete active).
    let mut state = AppState::default();
    let old_id = state.session.active_session.clone();
    state.active_chat_input_mut().insert_text("/new");

    // When handling SubmitMessage.
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then a new session is created.
    assert_ne!(state.session.active_session, old_id);
    // And the input buffer is cleared.
    assert!(state.active_chat_input().is_empty());
    // And no EnqueueUserMessage was emitted.
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
    assert_eq!(result.commands.len(), 1);
    assert!(
        matches!(&result.commands[0], Command::EnqueueUserMessage(..)),
        "unknown /command should be sent as chat"
    );
    // And the buffer is cleared.
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn submit_compact_slash_command_sends_compact_context() {
    // Given a state with "/compact" in the buffer.
    let mut state = AppState::default();
    let session_id = state.session.active_session.clone();
    state.active_chat_input_mut().insert_text("/compact");

    // When handling SubmitMessage.
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then a CompactContext command is dispatched.
    assert_eq!(result.commands.len(), 1);
    assert!(
        matches!(&result.commands[0], Command::CompactContext(cmd) if cmd.session_id == session_id),
        "/compact should send CompactContext command"
    );
    // And the buffer is cleared.
    assert!(state.active_chat_input().is_empty());
}

#[rstest::rstest]
fn tab_completes_name_without_executing() {
    // Given a state with slash autocomplete active ("/" typed, popup showing "new").
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);
    let old_id = state.session.active_session.clone();

    // When confirming autocomplete (Tab).
    let result = crate::feat::chat_input::intent::handle_autocomplete_confirm(&mut state);

    // Then the buffer contains "/new" (completed) but no session was created.
    assert_eq!(state.active_chat_input().text(), "/new");
    assert_eq!(
        state.session.active_session, old_id,
        "session should not change on Tab confirm"
    );
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn enter_completes_and_executes_slash_command() {
    // Given a state with slash autocomplete active ("/" typed, popup showing "new").
    let mut state = AppState::default();
    crate::feat::chat_input::intent::handle_insert_char('/', &mut state);
    let old_id = state.session.active_session.clone();

    // When pressing Enter (SubmitMessage with autocomplete active).
    let result = crate::feat::chat_input::intent::handle_submit_message(&mut state);

    // Then the command is completed and executed.
    assert_ne!(
        state.session.active_session, old_id,
        "session should change on Enter"
    );
    assert!(state.active_chat_input().is_empty());
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
    let result = crate::feat::chat_input::intent::handle_paste_text("hello\nworld", &mut state);

    // Then the buffer contains the pasted text with newlines preserved.
    assert_eq!(state.active_chat_input().text(), "hello\nworld");
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
    let result = crate::feat::chat_input::intent::handle_paste_text("XY", &mut state);

    // Then text is "heXYllo" and cursor is at 4.
    assert_eq!(state.active_chat_input().text(), "heXYllo");
    assert_eq!(state.active_chat_input().cursor_pos(), 4);
    assert!(result.commands.is_empty());
}
