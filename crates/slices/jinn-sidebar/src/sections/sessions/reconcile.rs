//! Sidebar-owned reconciliation after session removal.

use jinn_domain::common::app_state::AppState;

use super::state::sorted_open_sessions_split;

/// Reconciles cursor selection and active session after a removal completes.
pub fn reconcile_after_session_removal(state: &mut AppState) {
    reconcile_split(&mut state.session, &mut state.frontend);
}

/// Split-borrow reconciliation used by the sidebar state actor.
pub fn reconcile_split(
    session: &mut jinn_domain::common::session_map::SessionMap,
    frontend: &mut jinn_domain::feat::ui::frontend_state::FrontendState,
) {
    let sessions = sorted_open_sessions_split(session, frontend);
    if sessions.is_empty() {
        frontend.update_sections(|sections| sections.sessions.selected_index = Some(0));
        return;
    }
    let current = frontend
        .with_sections(|sections| sections.sessions.selected_index, || None)
        .unwrap_or(0);
    let clamped = current.min(sessions.len() - 1);
    frontend.update_sections(|sections| sections.sessions.selected_index = Some(clamped));
    let active_id = session.active_session_id();
    if !sessions.iter().any(|entry| &entry.id == active_id)
        && let Some(entry) = sessions.get(clamped)
    {
        session.set_active(entry.id.clone());
    }
}
