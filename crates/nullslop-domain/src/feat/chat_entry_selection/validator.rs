//! Chat entry selection validators.
//!
//! Validators for chat entry navigation and pinning intents.
//! Most are infallible; pin selected is fallible.

use crate::common::app_state::AppState;
use wherror::Error;

#[cfg(test)]
use crate::feat::session::tool_result_status::ToolResultStatus;

// --- Infallible validators ---

/// Validates the ChatEntrySelectNext intent.
pub fn validate_chat_entry_select_next(_state: &AppState) {}

/// Validates the ChatEntrySelectPrev intent.
pub fn validate_chat_entry_select_prev(_state: &AppState) {}

// --- Fallible validators ---

/// Errors from validating an ExpandToolEntry intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum ExpandToolEntryError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The selected entry is not a tool entry (tool call or tool result).
    NotToolEntry,
}

/// Validates the ExpandToolEntry intent.
///
/// Returns an error if no entry is selected or the selected entry is not a tool entry
/// (tool call or tool result).
///
/// # Errors
///
/// Returns an error if no entry is selected or the selected entry is not a tool entry.
pub fn validate_expand_tool_entry(state: &AppState) -> Result<(), ExpandToolEntryError> {
    let selected = state
        .active_session()
        .selected_entry()
        .ok_or(ExpandToolEntryError::NoSelection)?;
    if !matches!(
        selected.kind,
        crate::protocol::ChatEntryKind::ToolCall { .. }
            | crate::protocol::ChatEntryKind::ToolResult { .. }
    ) {
        return Err(ExpandToolEntryError::NotToolEntry);
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
    /// The selected entry kind cannot be pinned.
    NotPinnable,
}

/// Validates the ChatEntryPinSelected intent.
///
/// Returns an error if the history is empty, no entry is selected, or the
/// selected entry is not a pinnable kind (only User, Assistant, ToolResult,
/// and Skill entries can be pinned).
///
/// # Errors
///
/// Returns an error if the history is empty, no entry is selected, or the
/// selected entry is not pinnable.
pub fn validate_chat_entry_pin_selected(state: &AppState) -> Result<(), ChatEntryPinSelectedError> {
    if state.active_session().history().is_empty() {
        return Err(ChatEntryPinSelectedError::EmptyHistory);
    }
    let selected = state
        .active_session()
        .selected_entry()
        .ok_or(ChatEntryPinSelectedError::NoSelection)?;
    if !selected.is_pinnable() {
        return Err(ChatEntryPinSelectedError::NotPinnable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::protocol::ChatEntry;

    use super::*;

    #[rstest::rstest]
    fn pin_selected_rejects_info_entry() {
        // Given a state with a selected info entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::info(vec![ratatui::text::Line::from("welcome")]));
        state.active_session_mut().select_next_entry();

        // When validating pin selected.
        let result = validate_chat_entry_pin_selected(&state);

        // Then it returns NotPinnable error.
        assert!(matches!(
            result,
            Err(ChatEntryPinSelectedError::NotPinnable)
        ));
    }

    #[rstest::rstest]
    fn pin_selected_rejects_system_entry() {
        // Given a state with a selected system entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::system("status"));
        state.active_session_mut().select_next_entry();

        // When validating pin selected.
        let result = validate_chat_entry_pin_selected(&state);

        // Then it returns NotPinnable error.
        assert!(matches!(
            result,
            Err(ChatEntryPinSelectedError::NotPinnable)
        ));
    }

    #[rstest::rstest]
    fn expand_tool_entry_succeeds_with_selected_tool_result() {
        // Given a state with a selected tool result entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_result(
                "id",
                "bash",
                "output",
                ToolResultStatus::Success,
            ));
        state.active_session_mut().select_next_entry();

        // When validating expand tool entry.
        let result = validate_expand_tool_entry(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn expand_tool_entry_succeeds_with_selected_tool_call() {
        // Given a state with a selected tool call entry.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::tool_call(
            "id",
            "bash",
            "{\"cmd\": true}",
        ));
        state.active_session_mut().select_next_entry();

        // When validating expand tool entry.
        let result = validate_expand_tool_entry(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn expand_tool_entry_fails_with_no_selection() {
        // Given a state with empty history (no selection possible).
        let state = AppState::default();

        // When validating expand tool entry.
        let result = validate_expand_tool_entry(&state);

        // Then it returns NoSelection error.
        assert!(matches!(result, Err(ExpandToolEntryError::NoSelection)));
    }

    #[rstest::rstest]
    fn expand_tool_entry_fails_with_non_tool_entry() {
        // Given a state with a selected user entry (not a tool entry).
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When validating expand tool entry.
        let result = validate_expand_tool_entry(&state);

        // Then it returns NotToolEntry error.
        assert!(matches!(result, Err(ExpandToolEntryError::NotToolEntry)));
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
        // Given a state with empty history (no selection possible).
        let state = AppState::default();

        // When validating pin selected.
        let result = validate_chat_entry_pin_selected(&state);

        // Then it returns EmptyHistory error.
        assert!(matches!(
            result,
            Err(ChatEntryPinSelectedError::EmptyHistory)
        ));
    }
}
