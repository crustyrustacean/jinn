//! Chat entry selection validators.
//!
//! Validators for chat entry navigation and pinning intents.
//! Most are infallible; pin selected is fallible.

use jinn_kernel::AppState;
use wherror::Error;

#[cfg(test)]
use jinn_core_types::ToolResultStatus;

/// Validates the `select-next` route action.
pub fn validate_chat_entry_select_next(_state: &AppState) {}

/// Validates the `select-prev` route action.
pub fn validate_chat_entry_select_prev(_state: &AppState) {}

/// Errors from validating the `yank-selected` route action.
#[derive(Debug, Error)]
#[error(debug)]
pub enum YankSelectedError {
    /// No chat entry is currently selected.
    NoSelection,
}

/// Validates the `yank-selected` route action.
///
/// Returns an error if no entry is currently selected.
///
/// # Errors
///
/// Returns an error if no entry is currently selected.
pub fn validate_yank_selected(state: &AppState) -> Result<(), YankSelectedError> {
    state
        .active_session()
        .selected_entry()
        .ok_or(YankSelectedError::NoSelection)?;
    Ok(())
}

/// Errors from validating an ExpandEntry intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum ExpandEntryError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The selected entry is not expandable (not a tool entry, annotation, or compaction).
    NotExpandable,
}

/// Validates the `expand-tool-entry` route action.
///
/// Returns an error if no entry is selected or the selected entry is not expandable
/// (tool call, tool result, compaction, or annotation).
///
/// # Errors
///
/// Returns an error if no entry is selected or the selected entry is not expandable.
pub fn validate_expand_tool_entry(state: &AppState) -> Result<(), ExpandEntryError> {
    let selected = state
        .active_session()
        .selected_entry()
        .ok_or(ExpandEntryError::NoSelection)?;
    if !matches!(
        selected.kind,
        jinn_core_types::ChatEntryKind::ToolCall { .. }
            | jinn_core_types::ChatEntryKind::ToolResult { .. }
            | jinn_core_types::ChatEntryKind::Compaction { .. }
            | jinn_core_types::ChatEntryKind::Annotation { .. }
    ) {
        return Err(ExpandEntryError::NotExpandable);
    }
    Ok(())
}

/// Errors from validating the `fork-from-entry` route action.
#[derive(Debug, Error)]
#[error(debug)]
pub enum ForkFromEntryError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The chat history is empty.
    EmptyHistory,
}

/// Validates the `fork-from-entry` route action.
///
/// Returns an error if the history is empty or no entry is selected.
/// Any entry type can be forked from (unlike pinning, which restricts kinds).
///
/// # Errors
///
/// Returns an error if the history is empty or no entry is selected.
pub fn validate_fork_from_entry(state: &AppState) -> Result<(), ForkFromEntryError> {
    if state.active_session().history().is_empty() {
        return Err(ForkFromEntryError::EmptyHistory);
    }
    state
        .active_session()
        .selected_entry()
        .ok_or(ForkFromEntryError::NoSelection)?;
    Ok(())
}

/// Errors from validating a NewSessionFromEntry intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum NewSessionFromEntryError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The chat history is empty.
    EmptyHistory,
    /// The selected entry is not a User or Assistant entry.
    InvalidKind,
}

/// Validates the NewSessionFromEntry intent.
///
/// Returns an error if the history is empty, no entry is selected, or the
/// selected entry is not a User or Assistant entry. Only those two kinds
/// reliably work as the sole seed of a fresh session: other kinds are either
/// dropped from context by default (System, Actor, Error, Thinking, Transient)
/// or produce malformed message sequences (ToolCall, ToolResult, Compaction).
///
/// # Errors
///
/// Returns an error if the history is empty, no entry is selected, or the
/// selected entry is not a User or Assistant entry.
pub fn validate_new_session_from_entry(state: &AppState) -> Result<(), NewSessionFromEntryError> {
    if state.active_session().history().is_empty() {
        return Err(NewSessionFromEntryError::EmptyHistory);
    }
    let entry = state
        .active_session()
        .selected_entry()
        .ok_or(NewSessionFromEntryError::NoSelection)?;
    if !matches!(
        entry.kind,
        jinn_core_types::ChatEntryKind::User { .. } | jinn_core_types::ChatEntryKind::Assistant(_)
    ) {
        return Err(NewSessionFromEntryError::InvalidKind);
    }
    Ok(())
}

/// Errors from validating the `pin-selected` route action.
#[derive(Debug, Error)]
#[error(debug)]
pub enum PinSelectedError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The chat history is empty.
    EmptyHistory,
}

/// Validates the `pin-selected` route action.
///
/// Returns an error if the history is empty or no entry is selected.
/// All entry types can be pinned - pin overrides context state and forces
/// the entry into the assembled LLM prompt.
///
/// # Errors
///
/// Returns an error if the history is empty or no entry is selected.
pub fn validate_pin_selected(state: &AppState) -> Result<(), PinSelectedError> {
    if state.active_session().history().is_empty() {
        return Err(PinSelectedError::EmptyHistory);
    }
    let _selected = state
        .active_session()
        .selected_entry()
        .ok_or(PinSelectedError::NoSelection)?;
    Ok(())
}

/// Errors from validating the `ignore-selected` route action.
#[derive(Debug, Error)]
#[error(debug)]
pub enum ChatEntryIgnoreSelectedError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The chat history is empty.
    EmptyHistory,
    /// The selected entry is pinned (unpin first).
    IsPinned,
}

/// Validates the `ignore-selected` route action.
///
/// Returns an error if the history is empty, no entry is selected,
/// or the selected entry is pinned. All entry types can be toggled -
/// the `x` key always produces a visible gutter color change.
///
/// # Errors
///
/// Returns an error if the history is empty, no entry is selected,
/// or the entry is pinned.
pub fn validate_chat_entry_ignore_selected(
    state: &AppState,
) -> Result<(), ChatEntryIgnoreSelectedError> {
    if state.active_session().history().is_empty() {
        return Err(ChatEntryIgnoreSelectedError::EmptyHistory);
    }
    let selected = state
        .active_session()
        .selected_entry()
        .ok_or(ChatEntryIgnoreSelectedError::NoSelection)?;
    if selected.is_pinned() {
        return Err(ChatEntryIgnoreSelectedError::IsPinned);
    }
    Ok(())
}

/// Errors from validating a ChatEntryResetSelected intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum ChatEntryResetSelectedError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The chat history is empty.
    EmptyHistory,
    /// The selected entry is pinned (unpin first).
    IsPinned,
}

/// Validates the ChatEntryResetSelected intent.
///
/// Mirrors the `x` toggle rules: the history must be non-empty, an entry must
/// be selected, and pinned entries are rejected (resetting a pinned entry to
/// `Default` has no visible effect since pin dominates the override).
///
/// # Errors
///
/// Returns an error if the history is empty, no entry is selected,
/// or the entry is pinned.
pub fn validate_chat_entry_reset_selected(
    state: &AppState,
) -> Result<(), ChatEntryResetSelectedError> {
    if state.active_session().history().is_empty() {
        return Err(ChatEntryResetSelectedError::EmptyHistory);
    }
    let selected = state
        .active_session()
        .selected_entry()
        .ok_or(ChatEntryResetSelectedError::NoSelection)?;
    if selected.is_pinned() {
        return Err(ChatEntryResetSelectedError::IsPinned);
    }
    Ok(())
}

/// Errors from validating a ChatEntryIsolateSelected intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum ChatEntryIsolateSelectedError {
    /// No chat entry is currently selected.
    NoSelection,
    /// The chat history is empty.
    EmptyHistory,
    /// The highlighted entry is pinned (its override is already authoritative).
    IsPinned,
}

/// Validates the ChatEntryIsolateSelected intent.
///
/// Mirrors the `x` toggle rules: the history must be non-empty, an entry must
/// be highlighted (a cursor resting on a collapsed ignored block selects
/// nothing), and a pinned highlight is rejected (isolate would force-include
/// what the pin already forces in, silently doing nothing).
///
/// # Errors
///
/// Returns an error if the history is empty, no entry is selected,
/// or the highlighted entry is pinned.
pub fn validate_chat_entry_isolate_selected(
    state: &AppState,
) -> Result<(), ChatEntryIsolateSelectedError> {
    if state.active_session().history().is_empty() {
        return Err(ChatEntryIsolateSelectedError::EmptyHistory);
    }
    let selected = state
        .active_session()
        .selected_entry()
        .ok_or(ChatEntryIsolateSelectedError::NoSelection)?;
    if selected.is_pinned() {
        return Err(ChatEntryIsolateSelectedError::IsPinned);
    }
    Ok(())
}

#[cfg(test)]
mod fork_from_entry_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use jinn_core_types::ChatEntry;
    use jinn_kernel::AppState;

    use super::*;

    #[rstest::rstest]
    fn fork_from_entry_rejects_empty_history() {
        // Given an empty session.
        let state = AppState::default();

        // When validating fork from entry.
        let result = validate_fork_from_entry(&state);

        // Then validation fails with EmptyHistory.
        assert!(matches!(result, Err(ForkFromEntryError::EmptyHistory)));
    }

    #[rstest::rstest]
    fn fork_from_entry_rejects_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().clear_selection();

        // When validating fork from entry.
        let result = validate_fork_from_entry(&state);

        // Then validation fails with NoSelection.
        assert!(matches!(result, Err(ForkFromEntryError::NoSelection)));
    }

    #[rstest::rstest]
    fn fork_from_entry_accepts_selected_entry() {
        // Given a state with a selected entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When validating fork from entry.
        let result = validate_fork_from_entry(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }
}

#[cfg(test)]
mod new_session_from_entry_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use jinn_core_types::ChatEntry;
    use jinn_core_types::ToolResultStatus;
    use jinn_kernel::AppState;

    use super::*;

    #[rstest::rstest]
    fn new_session_from_entry_rejects_empty_history() {
        // Given an empty session.
        let state = AppState::default();

        // When validating new session from entry.
        let result = validate_new_session_from_entry(&state);

        // Then validation fails with EmptyHistory.
        assert!(matches!(
            result,
            Err(NewSessionFromEntryError::EmptyHistory)
        ));
    }

    #[rstest::rstest]
    fn new_session_from_entry_rejects_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().clear_selection();

        // When validating new session from entry.
        let result = validate_new_session_from_entry(&state);

        // Then validation fails with NoSelection.
        assert!(matches!(result, Err(NewSessionFromEntryError::NoSelection)));
    }

    #[rstest::rstest]
    fn new_session_from_entry_rejects_system_entry() {
        // Given a state with a selected System entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::system("status update"));
        state.active_session_mut().select_next_entry();

        // When validating new session from entry.
        let result = validate_new_session_from_entry(&state);

        // Then validation fails with InvalidKind.
        assert!(matches!(result, Err(NewSessionFromEntryError::InvalidKind)));
    }

    #[rstest::rstest]
    fn new_session_from_entry_rejects_tool_result_entry() {
        // Given a state with a selected ToolResult entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_result(
                "call-1",
                "bash",
                "ok",
                ToolResultStatus::Success,
            ));
        state.active_session_mut().select_next_entry();

        // When validating new session from entry.
        let result = validate_new_session_from_entry(&state);

        // Then validation fails with InvalidKind.
        assert!(matches!(result, Err(NewSessionFromEntryError::InvalidKind)));
    }

    #[rstest::rstest]
    fn new_session_from_entry_accepts_user_entry() {
        // Given a state with a selected User entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When validating new session from entry.
        let result = validate_new_session_from_entry(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn new_session_from_entry_accepts_assistant_entry() {
        // Given a state with a selected Assistant entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("a metaprompt"));
        state.active_session_mut().select_next_entry();

        // When validating new session from entry.
        let result = validate_new_session_from_entry(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }
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
    use jinn_core_types::ChatEntry;

    use super::*;

    #[rstest::rstest]
    fn pin_selected_succeeds_with_transient_entry() {
        // Given a state with a selected transient entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::transient("welcome"));
        state.active_session_mut().select_next_entry();

        // When validating pin selected.
        let result = validate_pin_selected(&state);

        // Then it succeeds (all entry types can be pinned).
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn pin_selected_succeeds_with_system_entry() {
        // Given a state with a selected system entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::system("status"));
        state.active_session_mut().select_next_entry();

        // When validating pin selected.
        let result = validate_pin_selected(&state);

        // Then it succeeds (all entry types can be pinned).
        assert!(result.is_ok());
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
    fn expand_entry_succeeds_with_selected_compaction() {
        // Given a state with a selected compaction entry.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry {
            id: jinn_core_types::ChatEntryId::new(),
            timing: jinn_core_types::EntryTiming::instant_now(),
            kind: jinn_core_types::ChatEntryKind::Compaction {
                summary: "summary".to_owned(),
                tokens_before: 100,
                tokens_after: 50,
                entries_compacted: 5,
                model_used: "test/model".to_owned(),
            },
            pin_position: None,
            context_override: jinn_core_types::ContextOverride::Default,
            context_history: Vec::new(),
            token_count: None,
        });
        state.active_session_mut().select_next_entry();

        // When validating expand tool entry.
        let result = validate_expand_tool_entry(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn expand_tool_entry_succeeds_with_selected_annotation() {
        // Given a state with a selected annotation entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::annotation(vec![jinn_provider::UrlCitation {
                url: "https://example.com/a".to_owned(),
                title: "Source A".to_owned(),
                content: None,
                start_index: None,
                end_index: None,
            }]));
        state.active_session_mut().select_next_entry();

        // When validating expand tool entry.
        let result = validate_expand_tool_entry(&state);

        // Then it succeeds (annotations are expandable).
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn expand_tool_entry_fails_with_no_selection() {
        // Given a state with empty history (no selection possible).
        let state = AppState::default();

        // When validating expand tool entry.
        let result = validate_expand_tool_entry(&state);

        // Then it returns NoSelection error.
        assert!(matches!(result, Err(ExpandEntryError::NoSelection)));
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

        // Then it returns NotExpandable error.
        assert!(matches!(result, Err(ExpandEntryError::NotExpandable)));
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
        let result = validate_pin_selected(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn pin_selected_fails_with_empty_history() {
        // Given a state with no history.
        let state = AppState::default();

        // When validating pin selected.
        let result = validate_pin_selected(&state);

        // Then it returns EmptyHistory error.
        assert!(matches!(result, Err(PinSelectedError::EmptyHistory)));
    }

    #[rstest::rstest]
    fn pin_selected_fails_with_no_selection() {
        // Given a state with empty history (no selection possible).
        let state = AppState::default();

        // When validating pin selected.
        let result = validate_pin_selected(&state);

        // Then it returns EmptyHistory error.
        assert!(matches!(result, Err(PinSelectedError::EmptyHistory)));
    }
}

#[cfg(test)]
mod yank_selected_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use jinn_core_types::ChatEntry;
    use jinn_kernel::AppState;

    use super::*;

    #[rstest::rstest]
    fn yank_selected_rejects_empty_history() {
        // Given an empty session.
        let state = AppState::default();

        // When validating yank selected.
        let result = validate_yank_selected(&state);

        // Then validation fails with NoSelection.
        assert!(matches!(result, Err(YankSelectedError::NoSelection)));
    }

    #[rstest::rstest]
    fn yank_selected_rejects_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().clear_selection();

        // When validating yank selected.
        let result = validate_yank_selected(&state);

        // Then validation fails with NoSelection.
        assert!(matches!(result, Err(YankSelectedError::NoSelection)));
    }

    #[rstest::rstest]
    fn yank_selected_accepts_selected_entry() {
        // Given a state with a selected entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When validating yank selected.
        let result = validate_yank_selected(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }
}

#[cfg(test)]
mod ignore_selected_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use jinn_core_types::{ChatEntry, PinPosition};
    use jinn_kernel::AppState;

    use super::*;

    #[rstest::rstest]
    fn ignore_selected_rejects_empty_history() {
        // Given an empty session.
        let state = AppState::default();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation fails with EmptyHistory.
        assert!(matches!(
            result,
            Err(ChatEntryIgnoreSelectedError::EmptyHistory)
        ));
    }

    #[rstest::rstest]
    fn ignore_selected_rejects_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().clear_selection();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation fails with NoSelection.
        assert!(matches!(
            result,
            Err(ChatEntryIgnoreSelectedError::NoSelection)
        ));
    }

    #[rstest::rstest]
    fn ignore_selected_rejects_pinned_entry() {
        // Given a state with a selected pinned entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello").with_pin(PinPosition::Top));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation fails with IsPinned.
        assert!(matches!(
            result,
            Err(ChatEntryIgnoreSelectedError::IsPinned)
        ));
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_system_entry() {
        // Given a state with a selected system entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::system("system prompt"));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds (all entry types can be toggled).
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_thinking_entry() {
        // Given a state with a selected thinking entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::thinking("thinking..."));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds (all entry types can be toggled).
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_transient_entry() {
        // Given a state with a selected transient entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::transient("ephemeral"));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds (all entry types can be toggled).
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_compaction_entry() {
        // Given a state with a selected compaction entry.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry {
            id: jinn_core_types::ChatEntryId::new(),
            timing: jinn_core_types::EntryTiming::instant_now(),
            kind: jinn_core_types::ChatEntryKind::Compaction {
                summary: "summary".to_owned(),
                tokens_before: 100,
                tokens_after: 50,
                entries_compacted: 5,
                model_used: "test/model".to_owned(),
            },
            pin_position: None,
            context_override: jinn_core_types::ContextOverride::Default,
            context_history: Vec::new(),
            token_count: None,
        });
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds (all entry types can be toggled).
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_user_entry() {
        // Given a state with a selected user entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_assistant_entry() {
        // Given a state with a selected assistant entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("hi"));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_tool_call() {
        // Given a state with a selected tool call entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_call("id", "bash", "ls"));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_tool_result() {
        // Given a state with a selected tool result entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_result(
                "id",
                "bash",
                "output",
                jinn_core_types::ToolResultStatus::Success,
            ));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_error_entry() {
        // Given a state with a selected error entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::error("something went wrong"));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn ignore_selected_accepts_actor_entry() {
        // Given a state with a selected actor entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::actor("source", "text"));
        state.active_session_mut().select_next_entry();

        // When validating ignore selected.
        let result = validate_chat_entry_ignore_selected(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn reset_selected_rejects_empty_history() {
        // Given an empty session.
        let state = AppState::default();

        // When validating reset selected.
        let result = validate_chat_entry_reset_selected(&state);

        // Then validation fails with EmptyHistory.
        assert!(matches!(
            result,
            Err(ChatEntryResetSelectedError::EmptyHistory)
        ));
    }

    #[rstest::rstest]
    fn reset_selected_rejects_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().clear_selection();

        // When validating reset selected.
        let result = validate_chat_entry_reset_selected(&state);

        // Then validation fails with NoSelection.
        assert!(matches!(
            result,
            Err(ChatEntryResetSelectedError::NoSelection)
        ));
    }

    #[rstest::rstest]
    fn reset_selected_rejects_pinned_entry() {
        // Given a state with a selected pinned entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello").with_pin(PinPosition::Top));
        state.active_session_mut().select_next_entry();

        // When validating reset selected.
        let result = validate_chat_entry_reset_selected(&state);

        // Then validation fails with IsPinned.
        assert!(matches!(result, Err(ChatEntryResetSelectedError::IsPinned)));
    }

    #[rstest::rstest]
    fn isolate_selected_rejects_empty_history() {
        // Given an empty session.
        let state = AppState::default();

        // When validating isolate selected.
        let result = validate_chat_entry_isolate_selected(&state);

        // Then validation fails with EmptyHistory.
        assert!(matches!(
            result,
            Err(ChatEntryIsolateSelectedError::EmptyHistory)
        ));
    }

    #[rstest::rstest]
    fn isolate_selected_rejects_no_selection() {
        // Given a state with entries but no selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state.active_session_mut().clear_selection();

        // When validating isolate selected.
        let result = validate_chat_entry_isolate_selected(&state);

        // Then validation fails with NoSelection.
        assert!(matches!(
            result,
            Err(ChatEntryIsolateSelectedError::NoSelection)
        ));
    }

    #[rstest::rstest]
    fn isolate_selected_rejects_pinned_entry() {
        // Given a state with a selected pinned entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello").with_pin(PinPosition::Top));
        state.active_session_mut().select_next_entry();

        // When validating isolate selected.
        let result = validate_chat_entry_isolate_selected(&state);

        // Then validation fails with IsPinned.
        assert!(matches!(
            result,
            Err(ChatEntryIsolateSelectedError::IsPinned)
        ));
    }

    #[rstest::rstest]
    fn isolate_selected_accepts_unpinned_selection() {
        // Given a state with a selected unpinned entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("a metaprompt"));
        state.active_session_mut().select_next_entry();

        // When validating isolate selected.
        let result = validate_chat_entry_isolate_selected(&state);

        // Then validation succeeds.
        assert!(result.is_ok());
    }
}
