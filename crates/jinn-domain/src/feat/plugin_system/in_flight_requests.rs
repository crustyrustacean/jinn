//! Dumb in-flight-request registry shared between sync and async plugin threads.
//!
//! Maps a plugin-chosen task string to a [`CancellationToken`]. The host does no
//! key namespacing and no request-kind detection. Domain-specific cleanup of
//! cancelled work (e.g. publishing `CancelStream` for an `llm_oneshot` child
//! session) is owned by a drop guard installed inside the request handler, not
//! by this registry.
//!
//! ## Threading
//!
//! Cloned cheaply (Arc-internal) into both [`crate::SyncPlugins`] (render/intent thread)
//! and the async plugin `ThreadState` (`plugin-async` thread). Both share one map.
//!
//! ## Lifecycle
//!
//! ```text
//! ctx.request(name, data, {task="x"})
//!   └─ register("x")          → creates token, supersedes any prior "x"
//!   └─ run_request select! { handler | token.cancelled() }
//!        ├─ handler wins → remove("x")
//!        └─ token wins   → drop handler future → handler's drop guard publishes CancelStream
//! ctx.cancel("x")  (sync or async side)
//!   └─ cancel("x")            → fires token (sync, thread-safe); cleanup runs on drop
//! ```

use std::sync::Arc;

use dashmap::DashMap;
use tokio_util::sync::CancellationToken;
/// One entry in the registry.
struct RequestEntry {
    token: CancellationToken,
}

/// Dumb, flat in-flight-request registry keyed by a plugin-chosen task string.
///
/// Single-token-per-key: registering at an occupied key cancels-and-replaces
/// the prior occupant. The plugin gets concurrency via distinct task strings.
#[derive(Clone)]
pub struct InFlightRequests(Arc<DashMap<String, RequestEntry>>);

impl InFlightRequests {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Register a new in-flight request under `task`.
    ///
    /// If an entry already exists at `task`, it is cancelled and removed first
    /// (cancel-and-replace). Returns the fresh token the caller should race
    /// against.
    pub fn register(&self, task: &str) -> CancellationToken {
        // Supersede any prior occupant: fire its token so its future aborts,
        // then drop the entry.
        if let Some((_, old)) = self.0.remove(task) {
            old.token.cancel();
        }
        let token = CancellationToken::new();
        self.0.insert(
            task.to_owned(),
            RequestEntry {
                token: token.clone(),
            },
        );
        token
    }

    /// Cancel the request at `task`: fire its token and remove the entry.
    ///
    /// This method stays synchronous and thread-safe — it only fires the
    /// token. Any cleanup of the spawned work (e.g. publishing `CancelStream`
    /// for an `llm_oneshot` child session) is owned by the request handler's
    /// drop guard, which fires when the generic `run_request` layer drops the
    /// handler future on cancel.
    pub fn cancel(&self, task: &str) {
        if let Some((_, entry)) = self.0.remove(task) {
            entry.token.cancel();
        }
    }

    /// Remove an entry without cancelling (used when a request completes normally).
    pub fn remove(&self, task: &str) {
        self.0.remove(task);
    }

    /// Returns `true` if `task` currently has an in-flight entry.
    #[cfg(test)]
    pub(crate) fn contains(&self, task: &str) -> bool {
        self.0.contains_key(task)
    }
}

impl Default for InFlightRequests {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;

    #[test]
    fn register_inserts_entry() {
        // Given an empty registry.
        let reg = InFlightRequests::new();

        // When registering a task.
        let _token = reg.register("enrich");

        // Then the registry contains the task.
        assert!(reg.contains("enrich"));
    }

    #[test]
    fn register_at_occupied_key_cancels_prior() {
        // Given a registry with one registered task.
        let reg = InFlightRequests::new();
        let first = reg.register("enrich");

        // When registering the same task again.
        let second = reg.register("enrich");

        // Then the first token is cancelled and the second is live.
        assert!(first.is_cancelled(), "prior token must be cancelled");
        assert!(!second.is_cancelled(), "new token must be live");
    }

    #[test]
    fn cancel_fires_token_and_removes_entry() {
        // Given a registered task.
        let reg = InFlightRequests::new();
        let token = reg.register("enrich");

        // When cancelling the task.
        reg.cancel("enrich");

        // Then the token is fired and the entry is gone.
        assert!(token.is_cancelled(), "token must be cancelled");
        assert!(!reg.contains("enrich"));
    }

    #[test]
    fn cancel_is_noop_for_unknown_task() {
        // Given an empty registry.
        let reg = InFlightRequests::new();

        // When cancelling an unknown task.
        reg.cancel("nope");

        // Then nothing happens (no panic, no entry inserted).
        assert!(!reg.contains("nope"));
    }
}
