//! A non-empty map of chat sessions.
//!
//! [`SessionMap`] wraps a `HashMap<SessionId, ChatSessionState>` and enforces
//! the invariant that at least one session always exists. The active session
//! ID always points to a valid entry. This makes `active_session()` and
//! `active_session_mut()` infallible — no `Option`, no `expect`.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::feat::session::chat_session::ChatSessionState;
use crate::protocol::SessionId;

use crate::common::app_state::SessionLoadGuard;

/// A non-empty map of sessions. The active session is always present.
///
/// # Invariants
///
/// - The map is never empty after construction.
/// - `active_session_id` always points to an existing entry.
/// - `remove()` creates a fresh session if the map would become empty.
#[derive(Debug)]
pub struct SessionMap {
    sessions: HashMap<SessionId, ChatSessionState>,
    active_session: SessionId,
    session_load_guard: Option<SessionLoadGuard>,
    default_cwd: PathBuf,
}

impl Default for SessionMap {
    fn default() -> Self {
        let session = ChatSessionState::new();
        let id = session.session_id().clone();
        let mut sessions = HashMap::new();
        sessions.insert(id.clone(), session);
        Self {
            sessions,
            active_session: id,
            session_load_guard: None,
            default_cwd: PathBuf::from("/"),
        }
    }
}

impl SessionMap {
    /// Create with an initial session. The map is never empty.
    pub fn new(session: ChatSessionState, default_cwd: PathBuf) -> Self {
        let id = session.session_id().clone();
        let mut sessions = HashMap::new();
        sessions.insert(id.clone(), session);
        Self {
            sessions,
            active_session: id,
            session_load_guard: None,
            default_cwd,
        }
    }

    /// Infallible — the active session is always present.
    #[expect(
        clippy::expect_used,
        reason = "SessionMap invariant: active_session always valid"
    )]
    pub fn active_session(&self) -> &ChatSessionState {
        self.sessions
            .get(&self.active_session)
            .expect("SessionMap invariant violation: active_session not in map")
    }

    /// Infallible mutable access — the active session is always present.
    #[expect(
        clippy::expect_used,
        reason = "SessionMap invariant: active_session always valid"
    )]
    pub fn active_session_mut(&mut self) -> &mut ChatSessionState {
        self.sessions
            .get_mut(&self.active_session)
            .expect("SessionMap invariant violation: active_session not in map")
    }

    /// The active session ID.
    pub fn active_session_id(&self) -> &SessionId {
        &self.active_session
    }

    /// Set the active session. Returns false if the ID doesn't exist.
    pub fn set_active(&mut self, id: SessionId) -> bool {
        if self.sessions.contains_key(&id) {
            self.active_session = id;
            true
        } else {
            false
        }
    }

    /// Fallible lookup by ID.
    pub fn get(&self, id: &SessionId) -> Option<&ChatSessionState> {
        self.sessions.get(id)
    }

    /// Fallible mutable lookup by ID.
    pub fn get_mut(&mut self, id: &SessionId) -> Option<&mut ChatSessionState> {
        self.sessions.get_mut(id)
    }

    /// Infallible lookup by ID. Use when the caller knows the session exists.
    ///
    /// # Panics
    ///
    /// Panics if the session does not exist. Prefer `get()` when the ID
    /// may not be present.
    #[expect(clippy::expect_used, reason = "caller guarantees session exists")]
    pub fn get_unchecked(&self, id: &SessionId) -> &ChatSessionState {
        self.sessions
            .get(id)
            .expect("session must exist in SessionMap")
    }

    /// Infallible mutable lookup by ID. Use when the caller knows the session exists.
    ///
    /// # Panics
    ///
    /// Panics if the session does not exist. Prefer `get_mut()` when the ID
    /// may not be present.
    #[expect(clippy::expect_used, reason = "caller guarantees session exists")]
    pub fn get_unchecked_mut(&mut self, id: &SessionId) -> &mut ChatSessionState {
        self.sessions
            .get_mut(id)
            .expect("session must exist in SessionMap")
    }

    /// Returns mutable access to a session by ID, creating it if missing.
    pub fn get_or_create(&mut self, id: &SessionId) -> &mut ChatSessionState {
        let default_cwd = self.default_cwd.clone();
        self.sessions.entry(id.clone()).or_insert_with(|| {
            let mut s = ChatSessionState::new();
            s.set_session_id(id.clone());
            s.set_cwd(default_cwd);
            s
        })
    }

    /// Insert a session.
    pub fn insert(&mut self, session: ChatSessionState) {
        self.sessions.insert(session.session_id().clone(), session);
    }

    /// Remove a session. If the map would become empty, creates a fresh session.
    /// If the removed session was active, switches to the next (or fresh) session.
    /// Returns true if the session was found and removed.
    pub fn remove(&mut self, id: &SessionId) -> bool {
        let removed = self.sessions.remove(id).is_some();
        if !removed {
            return false;
        }

        // If we removed the active session, switch to another.
        if id == &self.active_session {
            self.active_session = self.sessions.keys().next().cloned().unwrap_or_else(|| {
                // Map is empty — create a fresh session.
                let fresh = ChatSessionState::new();
                let fresh_id = fresh.session_id().clone();
                self.sessions.insert(fresh_id.clone(), fresh);
                fresh_id
            });
        }

        true
    }

    /// All sessions.
    pub fn sessions(&self) -> &HashMap<SessionId, ChatSessionState> {
        &self.sessions
    }

    /// Mutable access to all sessions.
    pub fn sessions_mut(&mut self) -> &mut HashMap<SessionId, ChatSessionState> {
        &mut self.sessions
    }

    /// Whether a session is currently being loaded from disk.
    pub fn is_loading(&self) -> bool {
        self.session_load_guard.is_some()
    }

    /// Begin loading a session. Sets the guard with the current timestamp.
    pub fn begin_load(&mut self, session_id: SessionId) {
        self.session_load_guard = Some(SessionLoadGuard {
            session_id,
            started_at: std::time::Instant::now(),
        });
    }

    /// Clear the loading guard (called on completion or timeout).
    pub fn clear_load(&mut self) {
        self.session_load_guard = None;
    }

    /// The loading guard, if active.
    pub fn session_load_guard(&self) -> Option<&SessionLoadGuard> {
        self.session_load_guard.as_ref()
    }

    /// The default CWD for new sessions.
    pub fn default_cwd(&self) -> &PathBuf {
        &self.default_cwd
    }

    /// Set the default CWD.
    pub fn set_default_cwd(&mut self, cwd: PathBuf) {
        self.default_cwd = cwd;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::common::app_state::AppState;

    fn default_map() -> SessionMap {
        let session = ChatSessionState::new();
        SessionMap::new(session, PathBuf::from("/"))
    }

    #[rstest::rstest]
    fn active_session_returns_valid_ref() {
        // Given a default session map.
        let map = default_map();

        // When accessing the active session.
        let session = map.active_session();

        // Then it returns a valid reference.
        assert_eq!(session.session_id(), map.active_session_id());
    }

    #[rstest::rstest]
    fn set_active_returns_false_for_unknown_id() {
        // Given a session map.
        let mut map = default_map();

        // When setting active to a non-existent ID.
        let result = map.set_active(SessionId::new());

        // Then it returns false.
        assert!(!result);
    }

    #[rstest::rstest]
    fn remove_last_session_creates_fresh() {
        // Given a map with one session.
        let mut map = default_map();
        let id = map.active_session_id().clone();

        // When removing the only session.
        let removed = map.remove(&id);

        // Then it was removed.
        assert!(removed);
        // And the map still has a session (fresh one created).
        assert!(!map.sessions.is_empty());
        assert!(map.active_session().session_id() != &id);
    }

    #[rstest::rstest]
    fn remove_non_active_preserves_active() {
        // Given a map with two sessions.
        let mut map = default_map();
        let active_id = map.active_session_id().clone();
        let other = ChatSessionState::new();
        let other_id = other.session_id().clone();
        map.insert(other);

        // When removing the non-active session.
        let removed = map.remove(&other_id);

        // Then the active session is unchanged.
        assert!(removed);
        assert_eq!(map.active_session_id(), &active_id);
    }

    #[rstest::rstest]
    fn remove_active_switches_to_next() {
        // Given a map with two sessions.
        let mut map = default_map();
        let other = ChatSessionState::new();
        let other_id = other.session_id().clone();
        map.insert(other);
        let active_id = map.active_session_id().clone();

        // When removing the active session.
        let removed = map.remove(&active_id);

        // Then active switches to the remaining session.
        assert!(removed);
        assert_eq!(map.active_session_id(), &other_id);
    }

    #[rstest::rstest]
    fn get_returns_none_for_unknown() {
        // Given a session map.
        let map = default_map();

        // When looking up a non-existent ID.
        let result = map.get(&SessionId::new());

        // Then it returns None.
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn get_or_create_creates_when_missing() {
        // Given a session map.
        let mut map = default_map();
        let new_id = SessionId::new();

        // When getting or creating a missing session.
        let session = map.get_or_create(&new_id);

        // Then a new session was created with that ID.
        assert_eq!(session.session_id(), &new_id);
    }
}
