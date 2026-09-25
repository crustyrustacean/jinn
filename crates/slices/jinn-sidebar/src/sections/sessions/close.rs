//! Session-close validation and lifecycle command flow.

use jinn_domain::common::app_state::AppState;
use jinn_domain::protocol::IntentResult;
use jinn_session_msg::PhaseKind;

use super::state::sorted_open_sessions;

/// Why a session close can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCloseError {
    /// The sessions section is not focused.
    WrongSection,
    /// No session is selected.
    NoSelection,
    /// The selected entry is not a session.
    NotASession,
    /// The selected session is streaming or sending.
    SessionBusy,
}

/// Validates that a session close can proceed.
///
/// # Errors
///
/// Returns [`SessionCloseError`] if the sessions section is not focused, no session is selected, or the session is busy.
pub fn validate_session_close(state: &AppState) -> Result<(), SessionCloseError> {
    if !matches!(
        state.frontend.sidebar_section(),
        Some(jinn_sidebar_msg::SidebarSectionId::Sessions)
    ) {
        return Err(SessionCloseError::WrongSection);
    }
    let index = state
        .frontend
        .with_sections(|sections| sections.sessions.selected_index, || None)
        .ok_or(SessionCloseError::NoSelection)?;
    let entries = sorted_open_sessions(state);
    let entry = entries
        .get(index)
        .ok_or(SessionCloseError::NoSelection)?;
    let session = state
        .session
        .get(&entry.id)
        .ok_or(SessionCloseError::NoSelection)?;
    if session.is_busy() || !matches!(session.phase(), PhaseKind::Idle) {
        return Err(SessionCloseError::SessionBusy);
    }
    Ok(())
}

/// Arms the close prompt on the first press and emits `CloseSession` on the
/// second press after re-validating the selected session.
pub fn handle_session_close_arm(state: &mut AppState) -> IntentResult {
    if state.frontend.close_session_prompt {
        state.frontend.close_session_prompt = false;
        return handle_session_close_with_lifecycle(state);
    }
    state.frontend.close_session_prompt = true;
    IntentResult::empty()
}

/// Emits the lifecycle close command for the selected session.
pub fn handle_session_close_with_lifecycle(state: &mut AppState) -> IntentResult {
    use jinn_session_lifecycle_msg::CloseSession;

    if validate_session_close(state).is_err() {
        return IntentResult::empty();
    }
    let index = state
        .frontend
        .with_sections(|sections| sections.sessions.selected_index, || None)
        .expect("validated session close has a selected index");
    let selected = sorted_open_sessions(state)
        .get(index)
        .expect("validated session close has a selected entry")
        .id
        .clone();
    IntentResult::new_message(CloseSession {
        session_id: selected,
    })
}
