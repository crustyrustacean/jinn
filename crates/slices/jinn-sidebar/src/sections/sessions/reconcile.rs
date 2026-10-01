//! Sidebar-owned reconciliation after session removal.

use jinn_kernel::common::app_state::AppState;

use super::state::sorted_open_sessions_split;

/// Reconciles cursor selection and active session after a removal completes.
///
/// `was_active` is what the publisher saw before the removal, because nothing
/// after the fact can say whether the user was reading the session that went:
/// by then the active id names whatever `remove_and_replace` moved it to.
pub fn reconcile_after_session_removal(state: &mut AppState, was_active: bool) {
    reconcile_split(&mut state.session, &mut state.frontend, was_active);
}

/// Split-borrow reconciliation used by the sidebar state actor.
pub fn reconcile_split(
    session: &mut jinn_session_state::SessionMap,
    frontend: &mut jinn_kernel::state::frontend_state::FrontendState,
    was_active: bool,
) {
    let sessions = sorted_open_sessions_split(session, frontend);
    if sessions.is_empty() {
        frontend.update_sections(|sections| sections.sessions.selected_id = None);
        return;
    }
    // A cursor names a session rather than an index, which is what lets a key
    // act on the session's actual subtree — but it means nothing about *where*
    // the cursor was survives its own removal, and the row it held is gone by
    // the time this runs. The row captured from the last rendered frame is the
    // only remaining record of it, so a cursor that no longer names a listed
    // session returns to the row now sitting where it was, clamped to the last
    // row of a list that just got shorter. One that still names a listed
    // session is left exactly where the user put it.
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
        let restored = super::super::capture_rows::restored_sessions_row_split(session, frontend);
        if let Some(entry) = sessions.get(restored) {
            let id = entry.id.clone();
            frontend.update_sections(|sections| sections.sessions.selected_id = Some(id));
        }
    }

    // Only a removal the user was reading decides where they are reading next.
    // Archiving a different session must not pull them out of the conversation
    // on screen — the map's own retarget already left that conversation
    // active, and the cursor moving elsewhere is not a request to follow it.
    if !was_active {
        return;
    }
    let cursor = frontend.with_sections(|sections| sections.sessions.selected_id.clone(), || None);
    if let Some(id) = cursor {
        session.set_active(id);
    }
}
