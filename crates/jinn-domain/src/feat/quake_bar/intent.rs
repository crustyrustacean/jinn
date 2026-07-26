//! Quake bar intent handlers — open, close, submit, scroll, and input editing.
//!
//! The input buffer is the synchronous side of the quake bar: the
//! `IntentHandler` edits it directly (mirroring `cwd_input`). The command log,
//! by contrast, is owned by the [`QuakeBarActor`](super::quake_bar_actor) —
//! `handle_submit` clears the input here and routes the text to the actor via
//! [`SubmitQuakeBarCommand`](super::command::SubmitQuakeBarCommand) so there is
//! exactly one writer of the log.

use crate::common::app_state::AppState;
use crate::common::focus::FocusScope;
use crate::feat::quake_bar::command::SubmitQuakeBarCommand;
use crate::protocol::IntentResult;

/// Opens the quake bar overlay.
///
/// Pushes `FocusScope::QuakeBar` onto the scope stack. The quake bar captures
/// all keystrokes while open; the only exit is ESC (`handle_close`).
pub fn handle_open(state: &mut AppState) -> IntentResult {
    state.frontend.scope_stack.push(FocusScope::QuakeBar);
    IntentResult::empty()
}

/// Closes the quake bar overlay.
///
/// Pops `FocusScope::QuakeBar` only when it is the current (top) scope. A no-op
/// otherwise — defensive, since the quake bar is an interrupting overlay that
/// should be the top scope whenever it is visible.
pub fn handle_close(state: &mut AppState) -> IntentResult {
    if matches!(state.frontend.scope_stack.current(), FocusScope::QuakeBar) {
        state.frontend.scope_stack.pop();
    }
    IntentResult::empty()
}

/// Submits the quake bar input into the command log.
///
/// Reads and trims the input text, clears the input buffer, and — if the text
/// is non-empty — emits a [`SubmitQuakeBarCommand`] so the
/// [`QuakeBarActor`](super::quake_bar_actor) appends it to the log. Empty input
/// is a no-op (no command emitted).
pub fn handle_submit(state: &mut AppState) -> IntentResult {
    let text = state.frontend.quake_bar.input.text.input.trim().to_owned();

    // Clear the input buffer regardless: the key was pressed, so reset the box.
    state.frontend.quake_bar.input = crate::feat::quake_bar::state::QuakeBarInput::default();

    if text.is_empty() {
        IntentResult::empty()
    } else {
        IntentResult::new_message(SubmitQuakeBarCommand { text })
    }
}

/// Scrolls the command log one line toward the oldest content.
pub fn handle_scroll_up(state: &mut AppState) -> IntentResult {
    state.frontend.quake_bar.log.scroll_up();
    IntentResult::empty()
}

/// Scrolls the command log one line toward the newest content.
pub fn handle_scroll_down(state: &mut AppState) -> IntentResult {
    state.frontend.quake_bar.log.scroll_down();
    IntentResult::empty()
}

/// Inserts a character at the cursor position.
pub fn handle_insert_char(state: &mut AppState, ch: char) -> IntentResult {
    state.frontend.quake_bar.input.text.insert_char(ch);
    IntentResult::empty()
}

/// Deletes the grapheme before the cursor.
pub fn handle_delete(state: &mut AppState) -> IntentResult {
    state.frontend.quake_bar.input.text.delete();
    IntentResult::empty()
}

/// Deletes the grapheme at/after the cursor (forward delete).
pub fn handle_delete_forward(state: &mut AppState) -> IntentResult {
    state.frontend.quake_bar.input.text.delete_forward();
    IntentResult::empty()
}

/// Moves the cursor one grapheme left.
pub fn handle_cursor_left(state: &mut AppState) -> IntentResult {
    state.frontend.quake_bar.input.text.cursor_left();
    IntentResult::empty()
}

/// Moves the cursor one grapheme right.
pub fn handle_cursor_right(state: &mut AppState) -> IntentResult {
    state.frontend.quake_bar.input.text.cursor_right();
    IntentResult::empty()
}

/// Moves the cursor to the start of the input.
pub fn handle_cursor_to_start(state: &mut AppState) -> IntentResult {
    state.frontend.quake_bar.input.text.cursor_pos = 0;
    IntentResult::empty()
}

/// Moves the cursor to the end of the input.
pub fn handle_cursor_to_end(state: &mut AppState) -> IntentResult {
    let len = state.frontend.quake_bar.input.text.input.len();
    state.frontend.quake_bar.input.text.cursor_pos = len;
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;
    use crate::common::app_state::AppState;

    #[test]
    fn open_pushes_quake_bar_scope() {
        // Given default app state (Normal scope on top).
        let mut state = AppState::default();

        // When opening the quake bar.
        handle_open(&mut state);

        // Then the top scope is QuakeBar.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::QuakeBar
        ));
    }

    #[test]
    fn close_pops_when_quake_bar_is_top() {
        // Given a state with the quake bar open.
        let mut state = AppState::default();
        handle_open(&mut state);

        // When closing.
        handle_close(&mut state);

        // Then the QuakeBar scope is no longer on top.
        assert!(!matches!(
            state.frontend.scope_stack.current(),
            FocusScope::QuakeBar
        ));
    }

    #[test]
    fn close_is_noop_when_quake_bar_not_top() {
        // Given a state in the default scope (quake bar not open).
        let mut state = AppState::default();
        let scope_before = state.frontend.scope_stack.current().clone();

        // When closing (defensively).
        handle_close(&mut state);

        // Then the scope is unchanged.
        assert_eq!(state.frontend.scope_stack.current(), &scope_before);
    }
    #[test]
    fn insert_char_appends_to_input() {
        // Given a quake bar with an empty input.
        let mut state = AppState::default();

        // When inserting a character.
        handle_insert_char(&mut state, 'x');

        // Then the input buffer holds that character.
        assert_eq!(state.frontend.quake_bar.input.text.input, "x");
    }

    #[test]
    fn submit_clears_input_buffer() {
        // Given a quake bar with typed input.
        let mut state = AppState::default();
        handle_insert_char(&mut state, 'h');
        handle_insert_char(&mut state, 'i');

        // When submitting.
        let _ = handle_submit(&mut state);

        // Then the input buffer is empty.
        assert!(state.frontend.quake_bar.input.text.input.is_empty());
    }

    #[test]
    fn submit_with_text_emits_submit_command() {
        // Given a quake bar with typed input.
        let mut state = AppState::default();
        handle_insert_char(&mut state, 'h');
        handle_insert_char(&mut state, 'i');

        // When submitting.
        let result = handle_submit(&mut state);

        // Then a SubmitQuakeBarCommand message was emitted.
        assert_eq!(result.message_names.len(), 1);
        assert!(
            result
                .message_names
                .first()
                .is_some_and(|name| name.ends_with("SubmitQuakeBarCommand"))
        );
    }

    #[test]
    fn submit_with_empty_input_emits_no_command() {
        // Given a quake bar with empty input.
        let mut state = AppState::default();

        // When submitting.
        let result = handle_submit(&mut state);

        // Then no command was emitted.
        assert!(result.message_names.is_empty());
    }
}
