//! Sidebar-owned reconciliation after session removal.

use jinn_kernel::common::app_state::AppState;

use super::state::sorted_open_sessions_split;

/// Reconciles cursor selection and active session after a removal completes.
pub fn reconcile_after_session_removal(state: &mut AppState) {
    reconcile_split(&mut state.session, &mut state.frontend);
}

/// Split-borrow reconciliation used by the sidebar state actor.
pub fn reconcile_split(
    session: &mut jinn_session_state::SessionMap,
    frontend: &mut jinn_kernel::state::frontend_state::FrontendState,
) {
    let sessions = sorted_open_sessions_split(session, frontend);
    if sessions.is_empty() {
        frontend.update_sections(|sections| sections.sessions.selected_id = None);
        return;
    }
    // A cursor names a session, so there is no index to run off the end of
    // the list and nothing to clamp. The one thing that can leave it stale is
    // the session it names being removed, which is this function's reason to
    // exist — so a cursor that no longer names a listed session falls back to
    // the first row, and one that still names one is left exactly where the
    // user put it.
    let still_listed = frontend.with_sections(
        |sections| {
            sections
                .sessions
                .selected_id
                .as_ref()
                .is_some_and(|id| sessions.iter().any(|entry| &entry.id == id))
        },
        || true,
    );
    if !still_listed {
        frontend.update_sections(|sections| {
            sections.sessions.selected_id = Some(sessions[0].id.clone())
        });
    }

    let cursor = frontend.with_sections(|sections| sections.sessions.selected_id.clone(), || None);
    let active_id = session.active_session_id();
    if !sessions.iter().any(|entry| &entry.id == active_id)
        && let Some(id) = cursor
    {
        session.set_active(id);
    }
}
