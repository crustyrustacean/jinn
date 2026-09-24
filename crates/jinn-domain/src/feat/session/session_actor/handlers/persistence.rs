//! Turn-path persistence — the session actor's own save and interaction mark.
//!
//! Load, fork, and archive now live in the `jinn-session-store` slice. The
//! turn/context actor still saves directly on its own path: enqueue, streaming,
//! tool calls, pin changes, and task-list updates all mutate the session and
//! must reach disk without routing a command through another actor.

use super::super::SessionPersistenceActor;
use crate::common::actor_deps::BusPublish;
use crate::feat::session::protocol::UserInteracted;

impl SessionPersistenceActor {
    /// Saves the current state of a session to disk.
    ///
    /// Clones the session inside `spawn_blocking` to avoid blocking the
    /// async runtime with a potentially expensive `ChatSessionState` clone
    /// (which includes the full `Vec<ChatEntry>` history). The store's
    /// `save` method does its own `spawn_blocking` internally for SQLite I/O.
    /// Errors are logged as warnings - persistence failure must not break
    /// the user experience.
    pub(in crate::feat::session::session_actor) async fn save_active_session(
        &self,
        session_id: &crate::protocol::SessionId,
    ) {
        let store = &self.services.session_store;

        let state = self.state.clone();
        let cap = self.cap;
        let session_id = session_id.clone();
        let session_id_log = session_id.clone();

        // The write lock is held only for the cheap `touch()` mutation, then
        // dropped before cloning the (potentially large) history. Cloning under
        // a held write lock would starve every other session's stream handlers,
        // which wait on `state.write_test_no_cap()` and backpressure the LLM stream consumer.
        let session = tokio::task::spawn_blocking(move || {
            {
                state.with_session(&cap, |view| {
                    if let Some(session) = view.session.map().get_mut(&session_id) {
                        session.touch();
                    }
                });
            }
            state.read().session.get(&session_id).cloned()
        })
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(err = ?e, "spawn_blocking panicked during session save");
            None
        });

        let Some(session) = session else { return };

        // Guard: don't persist sessions the user hasn't interacted with.
        if !session.is_persistable() {
            return;
        }

        if let Err(e) = store.save(&session).await {
            tracing::warn!(
                session_id = ?session_id_log,
                err = ?e,
                "failed to persist session"
            );
        }
    }

    /// Marks a session as having been interacted with by the user.
    ///
    /// Sets `has_interacted = true` on the session and emits a `UserInteracted` event.
    pub(in crate::feat::session::session_actor) async fn handle_mark_session_interacted(
        &mut self,
        payload: &crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted,
    ) {
        self.state.with_session(&self.cap, |view| {
            if let Some(session) = view.session.map().get_mut(&payload.session_id) {
                session.mark_interacted();
            }
        });

        self.publish(UserInteracted {
            session_id: payload.session_id.clone(),
        })
        .await;

        self.save_active_session(&payload.session_id).await;
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

    use super::super::super::helpers::test_actor_with_store_recording;

    #[rstest::rstest]
    #[tokio::test]
    async fn save_active_session_skips_non_persistable_session() {
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();

        actor.save_active_session(&session_id).await;

        assert!(
            store.last_saved_session(&session_id).is_none(),
            "non-interacted session should not be persisted"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_active_session_persists_interacted_session() {
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();
        {
            let mut state = actor.state.write_test_no_cap();
            state.active_session_mut().mark_interacted();
        }

        actor.save_active_session(&session_id).await;

        assert!(
            store.last_saved_session(&session_id).is_some(),
            "interacted session should be persisted"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_active_session_releases_write_lock_before_clone() {
        // Given an interacted session with a large history.
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();
        {
            let mut state = actor.state.write_test_no_cap();
            let session = state.active_session_mut();
            session.mark_interacted();
            // A large history makes the clone window wide enough to probe.
            for i in 0..5000 {
                session.push_entry(crate::protocol::ChatEntry::user(format!("msg {i}")));
            }
        }

        // When saving while a probe continuously attempts a read lock.
        // The probe records how many attempts were blocked by a writer.
        let probe_state = actor.state.clone();
        let probe = tokio::task::spawn_blocking(move || {
            let mut blocked = 0usize;
            let mut ok = 0usize;
            // Loop until the save's clone window has passed; bounded by total count.
            for _ in 0..100_000 {
                match probe_state.try_read() {
                    Some(_guard) => ok += 1,
                    None => blocked += 1,
                }
            }
            (ok, blocked)
        });

        actor.save_active_session(&session_id).await;
        let (ok, blocked) = probe.await.expect("probe panicked");

        // Then the session was saved (touch() ran under the write lock).
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "interacted session should be persisted"
        );
        // And the probe acquired a read lock many times, proving the write lock
        // is not held across the large history clone. A handful of transient
        // blocks (for touch()) is acceptable; thousands would indicate the bug.
        assert!(
            ok > 10_000,
            "readers starved during save: ok={ok}, blocked={blocked}"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn mark_session_interacted_marks_session_interacted() {
        // Given a session that has not been interacted with.
        let (mut actor, _store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();

        // When MarkSessionInteracted is handled.
        actor
            .handle_mark_session_interacted(
                &crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted {
                    session_id: session_id.clone(),
                },
            )
            .await;

        // Then the session is marked as interacted.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(session.has_interacted());
        assert!(session.is_persistable());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn mark_session_interacted_publishes_user_interacted() {
        // Given a session that has not been interacted with.
        let (mut actor, _store, audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();

        // When MarkSessionInteracted is handled.
        actor
            .handle_mark_session_interacted(
                &crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted {
                    session_id: session_id.clone(),
                },
            )
            .await;

        // Then the UserInteracted event is emitted.
        assert!(
            audit.contains_name("UserInteracted"),
            "UserInteracted event should be emitted"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn mark_session_interacted_persists_session() {
        // Given a session that has not been interacted with.
        let (mut actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();

        // When MarkSessionInteracted is handled.
        actor
            .handle_mark_session_interacted(
                &crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted {
                    session_id: session_id.clone(),
                },
            )
            .await;

        // Then the session is persisted.
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "interacted session should be persisted after MarkSessionInteracted"
        );
    }
}
