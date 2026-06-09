//! Session close validation and handlers.

use crate::common::app_state::AppState;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::ui::sidebar::sessions::state::{SessionEntryKind, sorted_open_sessions};

/// Why a session close can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCloseError {
    /// The sessions section is not focused.
    WrongSection,
    /// No session is selected.
    NoSelection,
    /// The selected entry is a plugin, not a session.
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
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;

    // Sessions section must be focused.
    if !matches!(
        state.frontend.scope_stack.sidebar_section(),
        Some(SidebarSectionId::Sessions)
    ) {
        return Err(SessionCloseError::WrongSection);
    }

    // A session must be selected.
    let index = state
        .frontend
        .sessions_section
        .selected_index
        .ok_or(SessionCloseError::NoSelection)?;

    // The selected session must be idle (not streaming/sending).
    let sessions = sorted_open_sessions(state);
    let entry = sessions.get(index).ok_or(SessionCloseError::NoSelection)?;

    // Plugin entries cannot be closed/archived.
    if matches!(entry.kind, SessionEntryKind::Plugin { .. }) {
        return Err(SessionCloseError::NotASession);
    }
    let session = state
        .session
        .get(&entry.id)
        .ok_or(SessionCloseError::NoSelection)?;
    if session.is_busy() || !matches!(session.phase(), PhaseKind::Idle) {
        return Err(SessionCloseError::SessionBusy);
    }

    Ok(())
}
/// Handles `SidebarSessionClose` - closes the selected session.
///
/// Removes the session from the in-memory HashMap (keeps it in SQLite).
/// Activates the next session in the sorted list, clamping the index.
/// If the last session is closed, creates a new empty session.
///
/// # Panics
///
/// Panics if the selected index is out of bounds (should not happen after validation).
pub fn handle_session_close(state: &mut AppState) -> crate::protocol::IntentResult {
    // Validate.
    if validate_session_close(state).is_err() {
        return crate::protocol::IntentResult::empty();
    }

    let index = state.frontend.sessions_section.selected_index.unwrap();
    let sessions = sorted_open_sessions(state);
    let Some(closing) = sessions.get(index) else {
        return crate::protocol::IntentResult::empty();
    };
    let closing_id = closing.id.clone();
    drop(sessions);

    // Update visual-parent index before removing the session
    // (need it in memory to resolve its parent chain).
    super::update_visual_parents_on_removal(state, &closing_id);

    // Remove and replace if last session.
    let was_last = state.session.session_count() == 1;
    if was_last {
        // Last session - create a new one with the last-used model.
        let new_session = {
            let model = state
                .frontend
                .app_state
                .last_model
                .clone()
                .unwrap_or_else(|| crate::feat::provider_infra::NO_PROVIDER_ID.to_owned());

            crate::feat::session::chat_session::ChatSessionState::new_with_profile(
                crate::feat::session::profile::SessionProfile::from_config(model),
            )
        };
        state.session.remove_and_replace(&closing_id, new_session);
    } else {
        state.session.remove(&closing_id);
    }

    super::reconcile_after_session_removal(state);

    crate::protocol::IntentResult::empty()
}

/// Handles `SidebarSessionClose` - closes the selected session.
///
/// Validates that the close can proceed, gets the selected session ID,
/// then emits a `CloseSession` command. The session actor handles teardown,
/// archival, removal, and emits `SessionClosed` for the sidebar actor to
/// clamp the cursor.
///
/// # Panics
///
/// Panics if `sessions_section.selected_index` is `None`.
pub fn handle_session_close_with_lifecycle(state: &mut AppState) -> crate::protocol::IntentResult {
    use crate::feat::session::protocol::close_session::CloseSession;
    use crate::protocol::Command;

    // Validate.
    if validate_session_close(state).is_err() {
        return crate::protocol::IntentResult::empty();
    }

    let index = state.frontend.sessions_section.selected_index.unwrap();
    let sessions = sorted_open_sessions(state);
    let closing_id = sessions[index].id.clone();

    // Emit CloseSession - the actor handles teardown, archive, and removal.
    crate::protocol::IntentResult::with_commands(vec![Command::CloseSession(CloseSession {
        session_id: closing_id,
    })])
}
