//! Sidebar-owned sessions-list state adapter.

use std::collections::HashSet;

use jinn_domain::common::app_state::AppState;
pub use jinn_session_list::{SessionEntry, SessionEntryKind};
pub use jinn_sidebar_msg::SessionsSectionState;

use jinn_session_msg::{PhaseKind, SessionOrigin};

/// Collects loaded sessions in visible depth-first order.
pub fn sorted_open_sessions(state: &AppState) -> Vec<SessionEntry> {
    sorted_open_sessions_split(&state.session, &state.frontend)
}

/// Split-borrow variant used by sidebar actors and other slice-owned adapters.
pub fn sorted_open_sessions_split(
    session: &jinn_session_state::SessionMap,
    frontend: &jinn_domain::feat::ui::frontend_state::FrontendState,
) -> Vec<SessionEntry> {
    let active_id = session.active_session_id();
    let entries = session
        .iter()
        .filter(|(_, session)| {
            session.session_state() == jinn_session_store_msg::SessionState::Loaded
        })
        .map(|(id, session)| SessionEntry {
            kind: SessionEntryKind::Session,
            id: id.clone(),
            title: session.title().unwrap_or("Untitled Session").to_owned(),
            is_active: id == active_id,
            created_at: *session.created_at(),
            is_idle: matches!(session.phase(), PhaseKind::Idle) && !session.is_busy(),
            last_entry_is_error: session.history().last().is_some_and(|entry| {
                matches!(&entry.kind, jinn_core_types::ChatEntryKind::Error(..))
            }),
            parent_id: session.parent_session().clone(),
            depth: 0,
            ancestor_continuations: vec![],
            is_last_child: false,
            is_subagent: session.origin() == SessionOrigin::Subagent,
            has_live_term: frontend
                .slices()
                .and_then(|slices| {
                    slices
                        .reader::<jinn_term_msg::TerminalTabState>(&jinn_term_msg::term_tabs_slot())
                })
                .is_some_and(|cell| cell.read().live_terms.contains(id)),
        })
        .collect();
    let visual_parents = frontend.with_sections(
        |sections| sections.sessions.visual_parents.clone(),
        std::collections::HashMap::new,
    );
    jinn_session_list::visible_session_tree(entries, &visual_parents)
}

/// Repairs visual parents before a sidebar-owned removal operation.
pub fn update_visual_parents_on_removal(
    state: &mut AppState,
    removed_id: &jinn_core_types::SessionId,
) {
    update_visual_parents_on_removal_split(&mut state.session, &mut state.frontend, removed_id);
}

/// Split-borrow variant of [`update_visual_parents_on_removal`].
pub fn update_visual_parents_on_removal_split(
    session: &mut jinn_session_state::SessionMap,
    frontend: &mut jinn_domain::feat::ui::frontend_state::FrontendState,
    removed_id: &jinn_core_types::SessionId,
) {
    let (removed_parent, loaded_ids, direct_child_ids) = {
        let Some(removed_session) = session.get(removed_id) else {
            return;
        };
        (
            removed_session.parent_session().clone(),
            session
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<HashSet<_>>(),
            session
                .iter()
                .filter(|(_, child)| child.parent_session().as_ref() == Some(removed_id))
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
        )
    };
    frontend.update_sections(|sections| {
        jinn_session_list::repair_visual_parents_on_removal(
            &mut sections.sessions.visual_parents,
            removed_id,
            removed_parent.as_ref(),
            &loaded_ids,
            direct_child_ids,
        );
    });
}

/// Repairs visual parents after a removed row is no longer in the session map.
pub fn repair_visual_parents_after_removal(
    session: &jinn_session_state::SessionMap,
    frontend: &mut jinn_domain::feat::ui::frontend_state::FrontendState,
    removed_id: &jinn_core_types::SessionId,
    removed_parent: Option<&jinn_core_types::SessionId>,
) {
    let (loaded_ids, direct_child_ids) = {
        (
            session
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<HashSet<_>>(),
            session
                .iter()
                .filter(|(_, child)| child.parent_session().as_ref() == Some(removed_id))
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
        )
    };
    frontend.update_sections(|sections| {
        jinn_session_list::repair_visual_parents_on_removal(
            &mut sections.sessions.visual_parents,
            removed_id,
            removed_parent,
            &loaded_ids,
            direct_child_ids,
        );
    });
}

/// Clears visual-parent bypasses after a session becomes visible again.
pub fn clear_visual_parents_on_load(state: &mut AppState, loaded_id: &jinn_core_types::SessionId) {
    clear_visual_parents_on_load_split(&mut state.frontend, loaded_id);
}

/// Split-borrow variant of [`clear_visual_parents_on_load`].
pub fn clear_visual_parents_on_load_split(
    frontend: &mut jinn_domain::feat::ui::frontend_state::FrontendState,
    loaded_id: &jinn_core_types::SessionId,
) {
    frontend.update_sections(|sections| {
        jinn_session_list::clear_visual_parents_on_load(
            &mut sections.sessions.visual_parents,
            loaded_id,
        );
    });
}
