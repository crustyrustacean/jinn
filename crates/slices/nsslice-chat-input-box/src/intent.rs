//! Chat input box intent handlers.
//!
//! Handles 13 chat-input intents:
//!
//! - **InsertChar** — inserts a character, manages autocomplete triggering/filtering/expansion.
//! - **DeleteGrapheme** — backspace with autocomplete awareness.
//! - **DeleteGraphemeForward** — forward delete with autocomplete awareness.
//! - **SubmitMessage** — validates, extracts text, resets buffer, returns `EnqueueUserMessage`.
//! - **AutocompleteConfirm** — confirms autocomplete selection or falls back to tab switch.
//! - **Cursor movement** (8 intents) — move cursor, optionally deactivating autocomplete.

use nullslop_component::chat_input_box::AutocompleteMatch;
use nullslop_component::chat_input_box::ChatInputBoxState;
use nullslop_component::prompt_template::PromptTemplateStore;
use nullslop_component::AppState;
use nullslop_protocol::chat_input::EnqueueUserMessage;
use nullslop_protocol::{Command, IntentResult};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::validator;

// --- Character input ---

/// Handles `InsertChar` — inserts a character and manages autocomplete.
pub fn handle_insert_char(ch: char, state: &mut AppState) -> IntentResult {
    let is_autocomplete_active = state.active_chat_input().autocomplete().is_some();

    if is_autocomplete_active {
        state.active_chat_input_mut().insert_grapheme_at_cursor(ch);

        match ch {
            ' ' => {
                state.active_chat_input_mut().deactivate_autocomplete();
            }
            '$' => {
                let Some(token_start) = state.active_chat_input().autocomplete_token_start()
                else {
                    return IntentResult::empty();
                };
                let cursor_before_insert = state.active_chat_input().cursor_pos() - 1;
                let filter: String = state
                    .active_chat_input()
                    .text()
                    .graphemes(true)
                    .enumerate()
                    .skip_while(|(i, _)| *i < token_start + 1)
                    .take_while(|(i, _)| *i < cursor_before_insert)
                    .map(|(_, g)| g)
                    .collect();
                if let Some(template) = state.context.prompt_templates.find_by_name(&filter) {
                    let body = template.body.clone();
                    state.active_chat_input_mut().expand_autocomplete(&body);
                } else {
                    state.active_chat_input_mut().deactivate_autocomplete();
                }
            }
            _ => {
                let filter = state
                    .active_chat_input()
                    .autocomplete_filter()
                    .unwrap_or_default();
                let matches = compute_matches(&state.context.prompt_templates, &filter);
                state
                    .active_chat_input_mut()
                    .update_autocomplete_matches(matches);
            }
        }
    } else {
        state.active_chat_input_mut().insert_grapheme_at_cursor(ch);

        if ch == '$' {
            let input = state.active_chat_input();
            if is_valid_trigger_position(input) {
                let token_start = input.cursor_pos() - 1;
                let matches = compute_matches(&state.context.prompt_templates, "");
                state
                    .active_chat_input_mut()
                    .activate_autocomplete(token_start, matches);
            }
        }
    }

    IntentResult::empty()
}

// --- Deletion ---

/// Handles `DeleteGrapheme` — backspace with autocomplete awareness.
pub fn handle_delete_grapheme(state: &mut AppState) -> IntentResult {
    let should_deactivate =
        if let Some(token_start) = state.active_chat_input().autocomplete_token_start() {
            state.active_chat_input().cursor_pos() <= token_start + 1
        } else {
            false
        };

    if should_deactivate {
        state.active_chat_input_mut().deactivate_autocomplete();
        state
            .active_chat_input_mut()
            .delete_grapheme_before_cursor();
    } else if state.active_chat_input().autocomplete().is_some() {
        state
            .active_chat_input_mut()
            .delete_grapheme_before_cursor();
        let filter = state
            .active_chat_input()
            .autocomplete_filter()
            .unwrap_or_default();
        let matches = compute_matches(&state.context.prompt_templates, &filter);
        state
            .active_chat_input_mut()
            .update_autocomplete_matches(matches);
    } else {
        state
            .active_chat_input_mut()
            .delete_grapheme_before_cursor();
    }

    IntentResult::empty()
}

/// Handles `DeleteGraphemeForward` — forward delete with autocomplete awareness.
pub fn handle_delete_grapheme_forward(state: &mut AppState) -> IntentResult {
    let token_start = state.active_chat_input().autocomplete_token_start();
    let cursor = state.active_chat_input().cursor_pos();

    if let Some(token_start) = token_start {
        if cursor == token_start {
            state.active_chat_input_mut().deactivate_autocomplete();
            state.active_chat_input_mut().delete_grapheme_after_cursor();
        } else {
            state.active_chat_input_mut().delete_grapheme_after_cursor();
            let should_deactivate = should_deactivate_on_cursor_move(state);
            if should_deactivate {
                state.active_chat_input_mut().deactivate_autocomplete();
            } else {
                let filter = state
                    .active_chat_input()
                    .autocomplete_filter()
                    .unwrap_or_default();
                let matches = compute_matches(&state.context.prompt_templates, &filter);
                state
                    .active_chat_input_mut()
                    .update_autocomplete_matches(matches);
            }
        }
    } else {
        state.active_chat_input_mut().delete_grapheme_after_cursor();
    }

    IntentResult::empty()
}

// --- Submission ---

/// Handles `SubmitMessage` — validates, extracts text, resets buffer, returns command.
pub fn handle_submit_message(state: &mut AppState) -> IntentResult {
    if validator::validate_submit_message(state).is_err() {
        return IntentResult::empty();
    }

    let text = state.active_chat_input().text().to_owned();
    let session_id = state.session.active_session.clone();
    state.active_chat_input_mut().reset();

    IntentResult::with_commands(vec![Command::EnqueueUserMessage {
        payload: EnqueueUserMessage { session_id, text },
    }])
}

// --- Autocomplete ---

/// Handles `AutocompleteConfirm` — confirms selection or falls back to tab switch.
pub fn handle_autocomplete_confirm(state: &mut AppState) -> IntentResult {
    if validator::validate_autocomplete_confirm(state).is_ok() {
        if let Some(selected) = state.active_chat_input().autocomplete_selected() {
            let name = selected.name.clone();
            state.active_chat_input_mut().complete_autocomplete(&name);
            let filter = state
                .active_chat_input()
                .autocomplete_filter()
                .unwrap_or_default();
            let matches = compute_matches(&state.context.prompt_templates, &filter);
            state
                .active_chat_input_mut()
                .update_autocomplete_matches(matches);
        }
        IntentResult::empty()
    } else {
        // Fallback: switch tab when autocomplete is inactive.
        state.frontend.active_tab = state.frontend.active_tab.next();
        IntentResult::empty()
    }
}

// --- Cursor movement ---

/// Handles `MoveCursorLeft` — moves cursor left, deactivates autocomplete if needed.
pub fn handle_move_cursor_left(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().move_cursor_left();
    let should_deactivate = should_deactivate_on_cursor_move(state);
    if should_deactivate {
        state.active_chat_input_mut().deactivate_autocomplete();
    }
    IntentResult::empty()
}

/// Handles `MoveCursorRight` — moves cursor right, deactivates autocomplete if needed.
pub fn handle_move_cursor_right(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().move_cursor_right();
    let should_deactivate = should_deactivate_on_cursor_move(state);
    if should_deactivate {
        state.active_chat_input_mut().deactivate_autocomplete();
    }
    IntentResult::empty()
}

/// Handles `MoveCursorToStart` — moves cursor to start, deactivates autocomplete.
pub fn handle_move_cursor_to_start(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().deactivate_autocomplete();
    state.active_chat_input_mut().move_cursor_to_start();
    IntentResult::empty()
}

/// Handles `MoveCursorToEnd` — moves cursor to end, deactivates autocomplete.
pub fn handle_move_cursor_to_end(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().deactivate_autocomplete();
    state.active_chat_input_mut().move_cursor_to_end();
    IntentResult::empty()
}

/// Handles `MoveCursorWordLeft` — moves cursor one word left, deactivates autocomplete.
pub fn handle_move_cursor_word_left(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().deactivate_autocomplete();
    state.active_chat_input_mut().move_cursor_word_left();
    IntentResult::empty()
}

/// Handles `MoveCursorWordRight` — moves cursor one word right, deactivates autocomplete.
pub fn handle_move_cursor_word_right(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().deactivate_autocomplete();
    state.active_chat_input_mut().move_cursor_word_right();
    IntentResult::empty()
}

/// Handles `MoveCursorUp` — moves up in autocomplete or moves cursor up.
pub fn handle_move_cursor_up(state: &mut AppState) -> IntentResult {
    if state.active_chat_input().autocomplete().is_some() {
        state.active_chat_input_mut().autocomplete_move_up();
    } else {
        state.active_chat_input_mut().move_cursor_up();
    }
    IntentResult::empty()
}

/// Handles `MoveCursorDown` — moves down in autocomplete or moves cursor down.
pub fn handle_move_cursor_down(state: &mut AppState) -> IntentResult {
    if state.active_chat_input().autocomplete().is_some() {
        state.active_chat_input_mut().autocomplete_move_down();
    } else {
        state.active_chat_input_mut().move_cursor_down();
    }
    IntentResult::empty()
}

// --- Helpers ---

fn is_valid_trigger_position(input: &ChatInputBoxState) -> bool {
    let dollar_pos = input.cursor_pos() - 1;
    if dollar_pos == 0 {
        return true;
    }
    input.grapheme_at(dollar_pos - 1) == Some(" ")
}

fn should_deactivate_on_cursor_move(state: &AppState) -> bool {
    let Some(ac) = state.active_chat_input().autocomplete() else {
        return false;
    };
    state.active_chat_input().cursor_pos() <= ac.token_start()
}

fn compute_matches(store: &PromptTemplateStore, filter: &str) -> Vec<AutocompleteMatch> {
    store
        .fuzzy_search(filter)
        .into_iter()
        .map(|t| AutocompleteMatch {
            name: t.name.clone(),
            description: t.description.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use nullslop_component::AppState;

    use super::*;

    #[rstest::rstest]
    fn insert_char_appends_to_buffer() {
        // Given a default AppState.
        let mut state = AppState::default();

        // When handling InsertChar('x').
        let result = super::handle_insert_char('x', &mut state);

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
        let result = super::handle_delete_grapheme(&mut state);

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
        let result = super::handle_delete_grapheme_forward(&mut state);

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
        let result = super::handle_submit_message(&mut state);

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
        let result = super::handle_submit_message(&mut state);

        // Then no commands are returned.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn autocomplete_confirm_falls_back_to_switch_tab() {
        // Given a state with no autocomplete active.
        let mut state = AppState::default();
        let prev_tab = state.frontend.active_tab;

        // When handling AutocompleteConfirm.
        let result = super::handle_autocomplete_confirm(&mut state);

        // Then the tab has advanced.
        assert_ne!(state.frontend.active_tab, prev_tab);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn move_cursor_left_moves_cursor() {
        // Given a state with "ab" in the buffer.
        let mut state = AppState::default();
        state.active_chat_input_mut().insert_text("ab");
        assert_eq!(state.active_chat_input().cursor_pos(), 2);

        // When handling MoveCursorLeft.
        let result = super::handle_move_cursor_left(&mut state);

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
        let result = super::handle_move_cursor_right(&mut state);

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
        let result = super::handle_move_cursor_to_start(&mut state);

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
        let result = super::handle_move_cursor_to_end(&mut state);

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
        let result = super::handle_move_cursor_word_left(&mut state);

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
        let result = super::handle_move_cursor_word_right(&mut state);

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
        let result = super::handle_move_cursor_up(&mut state);

        // Then no crash and no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn move_cursor_down_delegates_to_state() {
        // Given a default state.
        let mut state = AppState::default();
        state.active_chat_input_mut().insert_grapheme_at_cursor('a');

        // When handling MoveCursorDown.
        let result = super::handle_move_cursor_down(&mut state);

        // Then no crash and no commands.
        assert!(result.commands.is_empty());
    }
}
