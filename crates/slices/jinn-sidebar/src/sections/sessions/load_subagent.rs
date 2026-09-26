//! Opens the child subagent session linked to the selected `task` entry.

use jinn_core_types::{ChatEntryKind, SessionId};
use jinn_domain::common::app_state::AppState;
use jinn_domain::feat::ui::chat_log::activate_session;
use jinn_domain::protocol::IntentResult;
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

/// Activates a child session, measuring it if it needs measuring.
///
/// One call, whether the child is in memory or not. It used to branch: an
/// in-memory child was switched to with no measurement at all, which put a
/// large child's whole history on the render thread for a frame — the exact
/// freeze the layout workers exist to prevent, reachable by pressing Enter on a
/// subagent call.
pub fn handle_load_subagent_session(state: &mut AppState) -> IntentResult {
    let Ok(child_id) = validate_load_subagent_session(state) else {
        return IntentResult::empty();
    };
    // A child that is not in memory needs a full load, and the store actor
    // recognises that on its own: it is the only writer of the session map, so
    // it is the only place that knows what is loaded. The command says
    // "activate this" either way.
    activate_session(state, child_id, IntentResult::empty())
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

    #[rstest::rstest]
    fn handle_measures_an_in_memory_child() {
        // Given a selected task call whose child is already in memory.
        //
        // This used to switch to the child with no measurement at all, which put
        // its whole history on the render thread for a frame — the freeze the
        // layout workers exist to prevent, reachable by pressing Enter on a
        // subagent call.
        let child_id = SessionId::new();
        let mut state = state_with_selected([task_call(Some(child_id.clone()))]);
        let mut child = jinn_session_state::ChatSessionState::new();
        child.set_session_id(child_id.clone());
        child.push_entry(jinn_core_types::ChatEntry::user("subagent work"));
        state.session.insert(child);

        // When opening the subagent.
        let result = handle_load_subagent_session(&mut state);

        // Then the child is measured off the render thread like any other
        // activation, rather than laid out inline.
        assert!(
            result.message_names[0].ends_with("SessionLoadRequested"),
            "an in-memory child must still be measured, got {:?}",
            result.message_names
        );
        assert!(
            state.session.is_loading(),
            "the load guard must be armed so the next frame sees the indicator"
        );
    }

    #[rstest::rstest]
    fn handle_makes_the_in_memory_child_active() {
        // Given a selected task call whose child is already in memory.
        let child_id = SessionId::new();
        let mut state = state_with_selected([task_call(Some(child_id.clone()))]);
        let mut child = jinn_session_state::ChatSessionState::new();
        child.set_session_id(child_id.clone());
        child.push_entry(jinn_core_types::ChatEntry::user("subagent work"));
        state.session.insert(child);

        // When opening the subagent.
        handle_load_subagent_session(&mut state);

        // Then the child is the active session.
        assert_eq!(state.session.active_session_id(), &child_id);
    }
}
