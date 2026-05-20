//! State types for the sessions sidebar section.

use crate::common::app_state::AppState;
use crate::feat::session::chat_session::SessionPhase;

/// Sessions section cursor state — stored on `FrontendState`.
///
/// Tracks the selected index within the sorted open sessions list.
/// `None` means no cursor (section not focused).
#[derive(Debug, Clone, Default)]
pub struct SessionsSectionState {
    /// Index into the sorted open sessions list.
    pub selected_index: Option<usize>,
    /// Scroll offset: the first session entry index that is visible.
    pub scroll_offset: usize,
}

pub(crate) struct SessionEntry {
    pub(crate) id: crate::protocol::SessionId,
    pub(crate) title: String,
    pub(crate) is_active: bool,
    pub(crate) created_at: jiff::Timestamp,
    pub(crate) is_idle: bool,
    pub(crate) last_entry_is_error: bool,
}

/// Collects all loaded sessions sorted by `created_at` descending (newest first).
///
/// Only includes sessions with `SessionState::Loaded` — archived sessions
/// are not in the `SessionMap` and thus excluded automatically.
pub(crate) fn sorted_open_sessions(state: &AppState) -> Vec<SessionEntry> {
    let active_id = state.session.active_session_id();
    let mut entries: Vec<SessionEntry> = state
        .session
        .sessions()
        .iter()
        .filter(|(_, session)| {
            session.session_state() == crate::feat::session::chat_session::SessionState::Loaded
        })
        .map(|(id, session): (&_, &_)| SessionEntry {
            id: id.clone(),
            title: session.title().unwrap_or("Untitled Session").to_owned(),
            is_active: id == active_id,
            created_at: *session.created_at(),
            is_idle: matches!(session.phase(), SessionPhase::Idle),
            last_entry_is_error: session
                .history()
                .last()
                .is_some_and(|e| matches!(&e.kind, crate::protocol::ChatEntryKind::Error(..))),
        })
        .collect();
    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    entries
}
