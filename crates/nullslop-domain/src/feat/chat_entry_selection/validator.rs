//! Chat entry selection validators.
//!
//! Validators for chat entry navigation and pinning intents.
//! Most are infallible; pin selected is fallible.

use crate::common::app_state::AppState;
use wherror::Error;

// --- Infallible validators ---

/// Validates the ChatEntrySelectNext intent.
pub fn validate_chat_entry_select_next(_state: &AppState) {}

/// Validates the ChatEntrySelectPrev intent.
pub fn validate_chat_entry_select_prev(_state: &AppState) {}

// --- Fallible validators ---

/// Errors from validating an ExpandToolResult intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum ExpandToolResultError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The selected entry is not a tool result.
    NotToolResult,
}

/// Validates the ExpandToolResult intent.
///
/// Returns an error if no entry is selected or the selected entry is not a tool result.
///
/// # Errors
///
/// Returns an error if no entry is selected or the selected entry is not a tool result.
pub fn validate_expand_tool_result(state: &AppState) -> Result<(), ExpandToolResultError> {
    let selected = state
        .active_session()
        .selected_entry()
        .ok_or(ExpandToolResultError::NoSelection)?;
    if !matches!(
        selected.kind,
        crate::protocol::ChatEntryKind::ToolResult { .. }
    ) {
        return Err(ExpandToolResultError::NotToolResult);
    }
    Ok(())
}

/// Errors from validating a ChatEntryPinSelected intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum ChatEntryPinSelectedError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The chat history is empty.
    EmptyHistory,
}

/// Validates the ChatEntryPinSelected intent.
///
/// Returns an error if the history is empty or no entry is selected.
///
/// # Errors
///
/// Returns an error if the history is empty or no entry is selected.
pub fn validate_chat_entry_pin_selected(state: &AppState) -> Result<(), ChatEntryPinSelectedError> {
    if state.active_session().history().is_empty() {
        return Err(ChatEntryPinSelectedError::EmptyHistory);
    }
    if state.active_session().selected_entry_index().is_none() {
        return Err(ChatEntryPinSelectedError::NoSelection);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::protocol::ChatEntry;

    use super::*;

    #[rstest::rstest]
    fn expand_tool_result_succeeds_with_selected_tool_result() {
        // Given a state with a selected tool result entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_result("id", "bash", "output", true));
        state.active_session_mut().select_next_entry();

        // When validating expand tool result.
        let result = validate_expand_tool_result(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn expand_tool_result_fails_with_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));

        // When validating expand tool result.
        let result = validate_expand_tool_result(&state);

        // Then it returns NoSelection error.
        assert!(matches!(result, Err(ExpandToolResultError::NoSelection)));
    }

    #[rstest::rstest]
    fn expand_tool_result_fails_with_non_tool_result_entry() {
        // Given a state with a selected user entry (not a tool result).
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When validating expand tool result.
        let result = validate_expand_tool_result(&state);

        // Then it returns NotToolResult error.
        assert!(matches!(result, Err(ExpandToolResultError::NotToolResult)));
    }

    #[rstest::rstest]
    fn pin_selected_succeeds_with_selected_entry() {
        // Given a state with a history entry that is selected.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When validating pin selected.
        let result = validate_chat_entry_pin_selected(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn pin_selected_fails_with_empty_history() {
        // Given a state with no history.
        let state = AppState::default();

        // When validating pin selected.
        let result = validate_chat_entry_pin_selected(&state);

        // Then it returns EmptyHistory error.
        assert!(matches!(
            result,
            Err(ChatEntryPinSelectedError::EmptyHistory)
        ));
    }

    #[rstest::rstest]
    fn pin_selected_fails_with_no_selection() {
        // Given a state with history but no entry selected.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));

        // When validating pin selected.
        let result = validate_chat_entry_pin_selected(&state);

        // Then it returns NoSelection error.
        assert!(matches!(
            result,
            Err(ChatEntryPinSelectedError::NoSelection)
        ));
    }
}
