//! Shared application state accessible from any thread.
//!
//! [`State`] wraps [`AppState`] into a single shared reference.
//! Read and write guards provide access without exposing synchronization details.

use std::sync::Arc;

use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use super::app_state::AppState;

/// Shared application state accessible from any thread.
///
/// Wraps [`AppState`] so readers always see a consistent snapshot.
#[derive(Debug, Clone)]
pub struct State {
    /// The underlying shared, lock-protected application state.
    inner: Arc<RwLock<AppState>>,
}

/// Read guard for application data.
pub struct StateReadGuard<'a> {
    /// The underlying read lock guard.
    inner: RwLockReadGuard<'a, AppState>,
}

/// Write guard for application data.
pub struct StateWriteGuard<'a> {
    /// The underlying write lock guard.
    inner: RwLockWriteGuard<'a, AppState>,
}

impl State {
    /// Create a new State wrapping the given `AppState`.
    #[must_use]
    pub fn new(data: AppState) -> Self {
        Self {
            inner: Arc::new(RwLock::new(data)),
        }
    }

    /// Acquire a read lock on the state.
    pub fn read(&self) -> StateReadGuard<'_> {
        StateReadGuard {
            inner: self.inner.read(),
        }
    }

    /// Acquire a write lock on the state. Requires the [`IntentHandlerCap`] —
    /// the deliberate special-case owner. The IntentHandler is single-threaded
    /// (runs synchronously on the platform layer's main thread) and delegates
    /// to ~131 leaf handlers, so it keeps God-mode access. No concurrent actor
    /// can reach this method because they don't hold the cap.
    pub fn write(&self, _cap: &crate::common::tcaps::IntentHandlerCap) -> StateWriteGuard<'_> {
        StateWriteGuard {
            inner: self.inner.write(),
        }
    }

    /// TEST-ONLY write access — bypasses the cap requirement so tests across
    /// crates aren't burdened with threading a cap through every call site.
    ///
    /// Never call from production code. The name is deliberately grep-obvious
    /// so misuse is visible in review and `rg`.
    #[doc(hidden)]
    pub fn write_test_no_cap(&self) -> StateWriteGuard<'_> {
        StateWriteGuard {
            inner: self.inner.write(),
        }
    }

    /// Acquire a write lock, returning the raw parking_lot guard.
    ///
    /// This is the seam the TCaps projection layer hooks into: `with_*` methods
    /// call this, then split-borrow disjoint fields of `AppState` for projection.
    /// It stays scoped to `crate::common` so only the tcaps projection layer (in
    /// `common/tcaps/`) can reach it. Actors in `feat/` are outside `common/` and
    /// cannot call it. `inner` itself remains private; this method is the only
    /// write path for the tcaps projections.
    pub(in crate::common) fn write_lock(&self) -> parking_lot::RwLockWriteGuard<'_, AppState> {
        self.inner.write()
    }

    /// Non-blocking read attempt — test/diagnostic only.
    ///
    /// Returns `None` if a writer holds the lock. Used by tests that must
    /// *observe* contention rather than merely reason about it.
    #[cfg(test)]
    pub fn try_read(&self) -> Option<StateReadGuard<'_>> {
        self.inner.try_read().map(|inner| StateReadGuard { inner })
    }
}

impl std::ops::Deref for StateReadGuard<'_> {
    type Target = AppState;

    fn deref(&self) -> &AppState {
        &self.inner
    }
}

impl std::ops::Deref for StateWriteGuard<'_> {
    type Target = AppState;

    fn deref(&self) -> &AppState {
        &self.inner
    }
}

impl std::ops::DerefMut for StateWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut AppState {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use crate::protocol::ChatEntry;

    use super::*;

    #[rstest::rstest]
    fn state_read_returns_app_state() {
        // Given a State with a chat entry.
        let mut data = AppState::default();
        data.active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        let state = State::new(data);

        // When reading.
        let guard = state.read();

        // Then the entry is visible.
        assert_eq!(guard.active_session().history().len(), 1);
    }

    #[rstest::rstest]
    fn state_write_allows_mutation() {
        // Given a State.
        let state = State::new(AppState::default());

        // When writing and pushing an entry.
        {
            let mut guard = state.write_test_no_cap();
            guard
                .active_session_mut()
                .push_entry(ChatEntry::user("hello"));
        }

        // Then the entry appears on next read.
        let guard = state.read();
        assert_eq!(guard.active_session().history().len(), 1);
    }

    #[rstest::rstest]
    fn state_is_cloneable() {
        // Given a State.
        let state = State::new(AppState::default());

        // When cloning.
        let clone = state.clone();

        // Then both clones point to the same underlying data.
        {
            let mut guard = clone.write_test_no_cap();
            guard
                .active_session_mut()
                .push_entry(ChatEntry::user("shared"));
        }
        let guard = state.read();
        assert_eq!(guard.active_session().history().len(), 1);
    }
}
