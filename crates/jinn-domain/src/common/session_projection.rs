//! Synchronous mutation projections for the shared session state.
//!
//! The projections expose the existing narrow session operation wrappers while
//! keeping each mutation closure inside one application-state write lock.

use crate::common::state::State;
use crate::state::frontend_state::FrontendState;
use jinn_session_state::SessionMap;

/// Narrow write handle to the session collection.
pub struct SessionOps<'a>(&'a mut SessionMap);

impl SessionOps<'_> {
    /// Direct mutable access to the session collection.
    pub fn map(&mut self) -> &mut SessionMap {
        self.0
    }

    /// Store an archived session's immutable tree snapshot.
    pub fn insert_frozen_node(&mut self, node: jinn_session_state::FrozenTreeNode) {
        self.0.insert_frozen_node(node);
    }
}

/// Mutable session collection exposed by a session projection.
pub struct SessionView<'a> {
    /// Mutable session collection.
    pub session: SessionOps<'a>,
}

/// Combined mutation view for session and sidebar reconciliation.
pub struct SessionSidebarView<'a> {
    /// Mutable session collection.
    pub session: SessionOps<'a>,
    /// Mutable frontend state.
    pub frontend: &'a mut FrontendState,
}

/// Combined mutation view for session pin and sidebar pin reconciliation.
pub struct SessionPinsView<'a> {
    /// Mutable session collection.
    pub session: SessionOps<'a>,
    /// Mutable frontend state.
    pub frontend: &'a mut FrontendState,
}

impl State {
    /// Mutate the session collection through [`SessionView`].
    pub fn with_session<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut SessionView<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut SessionView {
            session: SessionOps(&mut app.session),
        })
    }

    /// Atomically mutate session and sidebar state.
    pub fn with_session_sidebar<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut SessionSidebarView<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut SessionSidebarView {
            session: SessionOps(&mut app.session),
            frontend: &mut app.frontend,
        })
    }

    /// Atomically mutate session pins and sidebar pin state.
    pub fn with_session_pins<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut SessionPinsView<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut SessionPinsView {
            session: SessionOps(&mut app.session),
            frontend: &mut app.frontend,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::indexing_slicing,
        reason = "test module indexes known test fixtures"
    )]
    use crate::common::app_state::AppState;
    use crate::protocol::ChatEntry;

    use super::State;

    #[rstest::rstest]
    fn session_projection_mutation_is_visible_on_next_read() {
        // Given a State with the default active session.
        let state = State::new(AppState::default());
        let before = state.read().active_session().history().len();

        // When appending an entry through the session projection.
        state.with_session(|view| {
            view.session
                .map()
                .active_session_mut()
                .push_entry(ChatEntry::system("hello"));
        });

        // Then the active session history contains the new entry.
        let after = state.read().active_session().history().len();
        assert_eq!(after, before + 1);
    }
}
