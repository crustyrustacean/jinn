//! Opens the child subagent session linked to the selected `task` entry.

use jinn_core_types::{ChatEntryKind, SessionId};
use jinn_domain::common::app_state::AppState;
use jinn_domain::protocol::IntentResult;
use jinn_session_store_msg::SessionLoadRequested;
use jinn_tools_msg::TASK_TOOL_NAME;
use wherror::Error;

/// Why a selected entry cannot be opened as a subagent session.
#[derive(Debug, Error)]
#[error(debug)]
pub enum LoadSubagentError {
    /// Nothing is selected in the chat history.
    NoSelection,
    /// The selected entry is not a `task` tool call or its result.
    NotTaskCall,
    /// The selected `task` call/result carries no child-session link.
    NoChildLink,
}

/// Resolves the child session linked to the selected `task` entry.
///
/// # Errors
///
/// Returns [`LoadSubagentError`] when no task pair is selected or its call
/// carries no child-session link.
pub fn validate_load_subagent_session(state: &AppState) -> Result<SessionId, LoadSubagentError> {
    let entry = state
        .active_session()
        .selected_entry()
        .ok_or(LoadSubagentError::NoSelection)?;
    let call_id = match &entry.kind {
        ChatEntryKind::ToolCall { id, name, .. } if name == TASK_TOOL_NAME => id,
        ChatEntryKind::ToolResult { id, name, .. } if name == TASK_TOOL_NAME => id,
        _ => return Err(LoadSubagentError::NotTaskCall),
    };
    state
        .active_session()
        .history()
        .iter()
        .rev()
        .find_map(|entry| match &entry.kind {
            ChatEntryKind::ToolCall {
                id, child_session, ..
            } if id == call_id => child_session.clone(),
            _ => None,
        })
        .ok_or(LoadSubagentError::NoChildLink)
}

/// Activates an in-memory child or requests its standard disk load.
pub fn handle_load_subagent_session(state: &mut AppState) -> IntentResult {
    let Ok(child_id) = validate_load_subagent_session(state) else {
        return IntentResult::empty();
    };
    if state.session.get(&child_id).is_some() {
        state.session.set_active(child_id);
        return IntentResult::empty();
    }
    state.session.begin_load(child_id.clone());
    IntentResult::new_message(SessionLoadRequested {
        session_id: child_id,
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code"
    )]
    use super::*;
    use jinn_core_types::{ChatEntry, ToolResultStatus};

    fn task_call(child_session: Option<SessionId>) -> ChatEntry {
        let mut entry = ChatEntry::tool_call("task-call", TASK_TOOL_NAME, "{}");
        if let ChatEntryKind::ToolCall {
            child_session: link,
            ..
        } = &mut entry.kind
        {
            *link = child_session;
        }
        entry
    }

    fn state_with_selected(entries: impl IntoIterator<Item = ChatEntry>) -> AppState {
        let mut state = AppState::default_with_scope_focus();
        for entry in entries {
            state.active_session_mut().push_entry(entry);
        }
        state.active_session_mut().select_prev_entry();
        state
    }

    #[rstest::rstest]
    fn validate_rejects_when_nothing_is_selected() {
        // Given an active session with no history.
        let state = AppState::default_with_scope_focus();

        // When validating.
        let result = validate_load_subagent_session(&state);

        // Then no selection is reported.
        assert!(matches!(result, Err(LoadSubagentError::NoSelection)));
    }

    #[rstest::rstest]
    fn validate_resolves_linked_task_result_to_child_session() {
        // Given a selected task result whose call links to a child.
        let child_id = SessionId::new();
        let state = state_with_selected([
            task_call(Some(child_id.clone())),
            ChatEntry::tool_result(
                "task-call",
                TASK_TOOL_NAME,
                "done",
                ToolResultStatus::Success,
            ),
        ]);

        // When validating.
        let result = validate_load_subagent_session(&state);

        // Then the child session is resolved.
        assert_eq!(result.expect("linked task"), child_id);
    }

    #[rstest::rstest]
    fn handle_activates_in_memory_child_session() {
        // Given a selected task call whose child is loaded.
        let child_id = SessionId::new();
        let mut state = state_with_selected([task_call(Some(child_id.clone()))]);
        state.session.get_or_create(&child_id);

        // When opening the subagent.
        let result = handle_load_subagent_session(&mut state);

        // Then the child becomes active without a bus message.
        assert_eq!(state.session.active_session_id(), &child_id);
        assert!(result.messages.is_empty());
    }

    #[rstest::rstest]
    fn handle_requests_disk_load_for_unloaded_child() {
        // Given a selected task call whose child is not loaded.
        let child_id = SessionId::new();
        let mut state = state_with_selected([task_call(Some(child_id.clone()))]);

        // When opening the subagent.
        let result = handle_load_subagent_session(&mut state);

        // Then the standard load request is emitted and the map enters loading.
        assert!(result.message_names[0].ends_with("SessionLoadRequested"));
        assert!(state.session.is_loading());
    }
}
