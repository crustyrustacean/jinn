//! State capture for the stateless context-assembly service.

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
    let tools = state
        .tool_registry()
        .map(|cell| cell.read().tools_for_session(session_id))
        .unwrap_or_default();

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
