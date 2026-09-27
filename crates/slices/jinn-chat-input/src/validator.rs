//! Chat input intent validators.
//!
//! Validators for message submission and autocomplete confirmation.

use jinn_kernel::AppState;
use wherror::Error;

/// Errors from validating a SubmitMessage intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum SubmitMessageError {
    /// The input buffer is empty.
    EmptyBuffer,
}

/// Validates the SubmitMessage intent.
///
/// Returns an error if autocomplete is active or the input buffer is empty.
///
/// # Errors
///
/// Returns an error if autocomplete is active or the input buffer is empty.
pub fn validate_submit_message(state: &AppState) -> Result<(), SubmitMessageError> {
    if state
        .active_session()
        .with_input(jinn_chat_input_msg::ChatInputBoxState::is_empty, || true)
    {
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
///
/// # Errors
///
/// Returns an error if no autocomplete session is active.
pub fn validate_autocomplete_confirm(state: &AppState) -> Result<(), AutocompleteConfirmError> {
    if state
        .active_session()
        .with_input(|i| i.autocomplete().is_none(), || true)
    {
        return Err(AutocompleteConfirmError::NotActive);
    }
    Ok(())
}

/// Validates the NormalEscape intent.
///
/// Escape in Normal mode can always proceed.
pub fn validate_normal_escape(_state: &AppState) {}
