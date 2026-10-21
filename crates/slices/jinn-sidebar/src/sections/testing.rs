//! Shared test fixtures for the sidebar sections.
//!
//! Test-facing: the sidebar's sections each built a differently-shaped
//! session fixture under the same name, so a reader could not tell which
//! shape a given test relied on. The constructors here name the shapes
//! instead, so the difference between "three extra sessions" and "three
//! sessions in total" is visible at the call site.
//!
//! The count semantics differ between the original fixtures and are
//! preserved exactly. Most of them kept the default pre-seeded session and
//! added to it, so their `count` was the *total*; one removed the default
//! session and created `count` of its own, so its `count` was also the
//! total but the default was gone. A merge that blurred that would
//! silently change what a test observes, so each shape keeps its own
//! constructor.
//!
//! Only [`state_with_session_ids`] counts *created* sessions rather than
//! the total, because it returns the ids of the sessions it created.

use jinn_core_types::ChatEntry;
use jinn_core_types::SessionId;
use jinn_kernel::common::app_state::AppState;
use jinn_session_state::ChatSessionState;

/// A state holding `total` sessions, each extra one carrying a user entry
/// reading `message for session {i}`.
///
/// The default pre-seeded session is kept, so `total` counts every session
/// the state holds.
#[must_use]
pub fn state_with_message_sessions(total: usize) -> AppState {
    let mut state = AppState::default_with_scope_focus();
    for i in 1..total {
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user(format!("message for session {i}")));
        state.session.insert(session);
    }
    state
}

/// A state holding `total` sessions, each extra one carrying a user entry
/// reading `message {i}`.
///
/// The default pre-seeded session is kept, so `total` counts every session
/// the state holds.
#[must_use]
pub fn state_with_plain_message_sessions(total: usize) -> AppState {
    let mut state = AppState::default_with_scope_focus();
    for i in 1..total {
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user(format!("message {i}")));
        state.session.insert(session);
    }
    state
}

/// A state holding `total` sessions, the extra ones carrying no entry.
///
/// The default pre-seeded session is kept, so `total` counts every session
/// the state holds.
#[must_use]
pub fn state_with_bare_sessions(total: usize) -> AppState {
    let mut state = AppState::default_with_scope_focus();
    for _ in 1..total {
        state.session.insert(ChatSessionState::new());
    }
    state
}

/// A state holding exactly `total` titled sessions, with the default
/// pre-seeded session removed.
///
/// Unlike the other constructors, this one *drops* the default session, so
/// the state holds exactly `total` sessions and every one of them is titled
/// `session {index}`.
#[must_use]
pub fn state_with_titled_sessions(total: usize) -> AppState {
    let mut state = AppState::default_with_scope_focus();
    let default_id = state.session.active_session_id().clone();
    state.session.remove_without_replacement(&default_id);
    for index in 0..total {
        let mut session = ChatSessionState::new();
        session.set_title(format!("session {index}"));
        state.session.insert(session);
    }
    state
}

/// A state holding `count` newly created sessions, each carrying one user
/// entry reading `hello`, plus their ids.
///
/// The default pre-seeded session is kept alongside them, so the state holds
/// one session more than `count`; the returned ids name the created ones.
/// The sessions section's preview width is narrowed so the preview path has
/// a stable width to lay out against.
#[must_use]
pub fn state_with_session_ids(count: usize) -> (AppState, Vec<SessionId>) {
    let mut state = AppState::default_with_scope_focus();
    let mut ids = Vec::new();
    for _ in 0..count {
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello"));
        ids.push(session.session_id().clone());
        state.session.insert(session);
    }
    state
        .frontend
        .update_sections(|s| s.sessions.preview_content_width = 40);
    (state, ids)
}
