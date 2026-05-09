//! Chat input intent validators.
//!
//! Validators for text editing, cursor movement, message submission, autocomplete,
//! and interrupt intents. Most are infallible; three are fallible.

use nullslop_component::AppState;
use nullslop_protocol::Mode;
use wherror::Error;

// --- Infallible validators ---

/// Validates the InsertChar intent.
pub fn validate_insert_char(_state: &AppState, _ch: char) {}

/// Validates the DeleteGrapheme intent.
pub fn validate_delete_grapheme(_state: &AppState) {}

/// Validates the DeleteGraphemeForward intent.
pub fn validate_delete_grapheme_forward(_state: &AppState) {}

/// Validates the MoveCursorLeft intent.
pub fn validate_move_cursor_left(_state: &AppState) {}

/// Validates the MoveCursorRight intent.
pub fn validate_move_cursor_right(_state: &AppState) {}

/// Validates the MoveCursorToStart intent.
pub fn validate_move_cursor_to_start(_state: &AppState) {}

/// Validates the MoveCursorToEnd intent.
pub fn validate_move_cursor_to_end(_state: &AppState) {}

/// Validates the MoveCursorWordLeft intent.
pub fn validate_move_cursor_word_left(_state: &AppState) {}

/// Validates the MoveCursorWordRight intent.
pub fn validate_move_cursor_word_right(_state: &AppState) {}

/// Validates the MoveCursorUp intent.
pub fn validate_move_cursor_up(_state: &AppState) {}

/// Validates the MoveCursorDown intent.
pub fn validate_move_cursor_down(_state: &AppState) {}

/// Validates the SetMode intent.
pub fn validate_set_mode(_state: &AppState, _mode: &Mode) {}

// --- Fallible validators ---

/// Errors from validating a SubmitMessage intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum SubmitMessageError {
    /// The input buffer is empty.
    EmptyBuffer,
    /// Autocomplete is active — complete instead of submit.
    AutocompleteActive,
}

/// Validates the SubmitMessage intent.
///
/// Returns an error if autocomplete is active or the input buffer is empty.
pub fn validate_submit_message(state: &AppState) -> Result<(), SubmitMessageError> {
    if state.active_chat_input().autocomplete().is_some() {
        return Err(SubmitMessageError::AutocompleteActive);
    }
    if state.active_chat_input().is_empty() {
        return Err(SubmitMessageError::EmptyBuffer);
    }
    Ok(())
}

/// Errors from validating an AutocompleteConfirm intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum AutocompleteConfirmError {
    /// No autocomplete session is active.
    NotActive,
}

/// Validates the AutocompleteConfirm intent.
///
/// Returns an error if no autocomplete session is active.
pub fn validate_autocomplete_confirm(state: &AppState) -> Result<(), AutocompleteConfirmError> {
    if state.active_chat_input().autocomplete().is_none() {
        return Err(AutocompleteConfirmError::NotActive);
    }
    Ok(())
}

/// Errors from validating an Interrupt intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum InterruptError {
    /// Nothing to interrupt — buffer is empty and session is idle.
    NothingToInterrupt,
}

/// Validates the Interrupt intent.
///
/// Returns an error if the buffer is empty and the session is idle
/// (not streaming, not sending, not assembling).
pub fn validate_interrupt(state: &AppState) -> Result<(), InterruptError> {
    if state.active_chat_input().is_empty() && state.active_session().is_idle() {
        return Err(InterruptError::NothingToInterrupt);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Infallible validator tests ---

    #[rstest::rstest]
    fn validate_insert_char_always_succeeds() {
        let state = AppState::default();
        validate_insert_char(&state, 'x');
    }

    #[rstest::rstest]
    fn validate_delete_grapheme_always_succeeds() {
        let state = AppState::default();
        validate_delete_grapheme(&state);
    }

    #[rstest::rstest]
    fn validate_delete_grapheme_forward_always_succeeds() {
        let state = AppState::default();
        validate_delete_grapheme_forward(&state);
    }

    #[rstest::rstest]
    fn validate_move_cursor_left_always_succeeds() {
        let state = AppState::default();
        validate_move_cursor_left(&state);
    }

    #[rstest::rstest]
    fn validate_move_cursor_right_always_succeeds() {
        let state = AppState::default();
        validate_move_cursor_right(&state);
    }

    #[rstest::rstest]
    fn validate_move_cursor_to_start_always_succeeds() {
        let state = AppState::default();
        validate_move_cursor_to_start(&state);
    }

    #[rstest::rstest]
    fn validate_move_cursor_to_end_always_succeeds() {
        let state = AppState::default();
        validate_move_cursor_to_end(&state);
    }

    #[rstest::rstest]
    fn validate_move_cursor_word_left_always_succeeds() {
        let state = AppState::default();
        validate_move_cursor_word_left(&state);
    }

    #[rstest::rstest]
    fn validate_move_cursor_word_right_always_succeeds() {
        let state = AppState::default();
        validate_move_cursor_word_right(&state);
    }

    #[rstest::rstest]
    fn validate_move_cursor_up_always_succeeds() {
        let state = AppState::default();
        validate_move_cursor_up(&state);
    }

    #[rstest::rstest]
    fn validate_move_cursor_down_always_succeeds() {
        let state = AppState::default();
        validate_move_cursor_down(&state);
    }

    #[rstest::rstest]
    fn validate_set_mode_always_succeeds() {
        let state = AppState::default();
        validate_set_mode(&state, &Mode::Input);
    }

    // --- SubmitMessage tests ---

    #[rstest::rstest]
    fn submit_message_succeeds_with_non_empty_buffer() {
        // Given a state with text in the input buffer.
        let mut state = AppState::default();
        state.active_chat_input_mut().insert_grapheme_at_cursor('h');

        // When validating submit message.
        let result = validate_submit_message(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn submit_message_fails_with_empty_buffer() {
        // Given a state with an empty input buffer.
        let state = AppState::default();

        // When validating submit message.
        let result = validate_submit_message(&state);

        // Then it returns EmptyBuffer error.
        assert!(matches!(result, Err(SubmitMessageError::EmptyBuffer)));
    }

    #[rstest::rstest]
    fn submit_message_fails_when_autocomplete_active() {
        // Given a state with text and autocomplete active.
        let mut state = AppState::default();
        state.active_chat_input_mut().insert_grapheme_at_cursor('h');
        state
            .active_chat_input_mut()
            .activate_autocomplete(0, vec![]);

        // When validating submit message.
        let result = validate_submit_message(&state);

        // Then it returns AutocompleteActive error.
        assert!(matches!(
            result,
            Err(SubmitMessageError::AutocompleteActive)
        ));
    }

    // --- AutocompleteConfirm tests ---

    #[rstest::rstest]
    fn autocomplete_confirm_succeeds_when_active() {
        // Given a state with autocomplete active.
        let mut state = AppState::default();
        state
            .active_chat_input_mut()
            .activate_autocomplete(0, vec![]);

        // When validating autocomplete confirm.
        let result = validate_autocomplete_confirm(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn autocomplete_confirm_fails_when_not_active() {
        // Given a state with no autocomplete.
        let state = AppState::default();

        // When validating autocomplete confirm.
        let result = validate_autocomplete_confirm(&state);

        // Then it returns NotActive error.
        assert!(matches!(result, Err(AutocompleteConfirmError::NotActive)));
    }

    // --- Interrupt tests ---

    #[rstest::rstest]
    fn interrupt_succeeds_with_non_empty_buffer() {
        // Given a state with text in the input buffer and idle session.
        let mut state = AppState::default();
        state.active_chat_input_mut().insert_grapheme_at_cursor('h');

        // When validating interrupt.
        let result = validate_interrupt(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn interrupt_succeeds_with_active_stream() {
        // Given a state with empty buffer but an active stream.
        let mut state = AppState::default();
        state.active_session_mut().begin_streaming();

        // When validating interrupt.
        let result = validate_interrupt(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn interrupt_fails_with_empty_buffer_and_idle_session() {
        // Given a state with empty buffer and idle session.
        let state = AppState::default();

        // When validating interrupt.
        let result = validate_interrupt(&state);

        // Then it returns NothingToInterrupt error.
        assert!(matches!(result, Err(InterruptError::NothingToInterrupt)));
    }
}
