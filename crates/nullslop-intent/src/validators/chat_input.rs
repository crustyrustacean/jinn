//! Chat input intent validators.
//!
//! Validators for message submission, autocomplete confirmation, and interrupt intents.

use nullslop_component::AppState;
use wherror::Error;

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
