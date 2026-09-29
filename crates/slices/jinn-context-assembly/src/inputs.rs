//! State capture for the stateless context-assembly service.

use jinn_attendant_msg::is_attendant_tool;
use jinn_context_assembly_msg::AssemblyInputs;
use jinn_core_types::{DEFAULT_PERSONA_NAME, SessionId};
use jinn_kernel::common::app_state::AppState;
use jinn_session_state::AssemblySessionProjection;

/// Builds a coherent assembly request from the current application state.
///
/// The caller must hold the `State` read guard while invoking this function.
/// Session-owned values are captured together through
/// [`AssemblySessionProjection`]; persona and tool definitions are resolved
/// from their attached cells while the same guard is still held.
///
/// Missing persona or tool cells degrade to empty assembly values rather than
/// failing turn dispatch.
#[must_use]
pub fn build_assembly_inputs(state: &AppState, session_id: &SessionId) -> AssemblyInputs {
    let session = AssemblySessionProjection::capture(session_id, state.session(session_id));

    let persona = state.persona_selection().and_then(|cell| {
        cell.read()
            .resolve_for(&session.persona_name, DEFAULT_PERSONA_NAME)
            .cloned()
    });
    // Attendant-only tools are filtered per session, the same way server
    // tools are filtered per provider in `assemble`: they are dropped
    // before the prompt is built, so a session that is not an attendant
    // never sees them offered rather than being offered and refused.
    let tools = state
        .tool_registry()
        .map(|cell| cell.read().tools_for_session(session_id))
        .unwrap_or_default()
        .into_iter()
        .filter(|def| !is_attendant_tool(&def.name) || session.is_attendant)
        .collect();

    AssemblyInputs {
        session_id: session.session_id,
        cwd: session.cwd,
        persona,
        history: session.history,
        tools,
        disabled_tools: session.disabled_tools,
        provider_name: session.provider_name,
        skills: session.skills,
        disabled_skills: session.disabled_skills,
        loaded_skills: session.loaded_skills,
        context_files: session.context_files,
    }
}

#[cfg(test)]
mod attendant_tool_visibility_tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, reason = "test module")]

    use super::*;
    use jinn_attendant_msg::ATTENDANT_TOOL_NAMES;
    use jinn_core_types::tool_types::ToolDefinition;
    use jinn_kernel::common::app_state::AppState;
    use jinn_kernel::common::state::State;
    use jinn_session_state::ChatSessionState;

    #[rstest::rstest]
    #[test]
    fn an_ordinary_session_is_offered_no_attendant_tool() {
        // Given a plain user session with the built-ins registered.
        let (state, session_id) = state_with_session(ChatSessionState::new());

        // When its assembly inputs are built.
        let names = tool_names(&state, &session_id);

        // Then neither attendant tool appears. A user session is not a
        // judge and has no parent to wake, so offering the tools invites
        // calls that can only fail.
        for name in ATTENDANT_TOOL_NAMES {
            assert!(
                !names.iter().any(|n| n == name),
                "{name} must not be offered to a non-attendant session"
            );
        }
        // And the other built-ins are untouched.
        assert!(names.contains(&"read".to_owned()));
    }

    #[rstest::rstest]
    #[test]
    fn an_attendant_is_offered_both_of_its_tools() {
        // Given an attendant session.
        let parent = ChatSessionState::new();
        let (state, session_id) =
            state_with_session(ChatSessionState::new_attendant(&parent, true));

        // When its assembly inputs are built.
        let names = tool_names(&state, &session_id);

        // Then both are present. Filtering must not starve the tools an
        // attendant exists to use.
        for name in ATTENDANT_TOOL_NAMES {
            assert!(
                names.iter().any(|n| n == name),
                "{name} must reach an attendant"
            );
        }
    }

    /// A state holding `session` as its active session, with the
    /// attendant built-ins and one ordinary tool registered.
    fn state_with_session(session: ChatSessionState) -> (State, SessionId) {
        let state = State::new(AppState::default_with_scope_focus());
        let session_id = session.session_id().clone();
        {
            let mut guard = state.write();
            guard.session.insert(session);
            let cell = guard.tool_registry().expect("registry cell attached");
            cell.update(|r| {
                r.global.insert("report".to_owned(), tool("report"));
                r.global
                    .insert("notify_parent".to_owned(), tool("notify_parent"));
                r.global.insert("read".to_owned(), tool("read"));
            });
        }
        (state, session_id)
    }

    /// The tool names a session's assembly inputs carry.
    fn tool_names(state: &State, session_id: &SessionId) -> Vec<String> {
        let guard = state.read();
        build_assembly_inputs(&guard, session_id)
            .tools
            .iter()
            .map(|t| t.name.clone())
            .collect()
    }

    /// A minimal tool definition under `name`.
    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_owned(),
            description: String::new(),
            parameters: serde_json::json!({ "type": "object" }),
            prompt_snippet: None,
            prompt_guidelines: vec![],
            server_tool_type: None,
        }
    }
}
