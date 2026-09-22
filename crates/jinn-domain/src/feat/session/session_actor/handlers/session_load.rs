//! Session load and fork handlers - restore sessions from disk and fork session history.
//!
//! Handles restoring a loaded session into active state (validating CWD, emitting
//! follow-up commands for strategy restoration) and forking a session at a specific
//! point in its history.

use crate::common::actor_deps::BusPublish;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::protocol::ChatEntry;
use crate::protocol::system::ActiveSessionChanged;

use super::super::SessionPersistenceActor;
use crate::SessionForkRequested;

impl SessionPersistenceActor {
    /// SessionLoadCompleted: restore session state and emit follow-up commands.
    ///
    /// Carries the fully loaded [`ChatSessionState`]. This handler inserts it
    /// into state, restores model/CWD, validates the CWD, and recalculates
    /// context size.
    pub(in crate::feat::session::session_actor) async fn handle_session_load_completed(
        &self,
        payload: &SessionLoadCompleted,
    ) {
        let session_id = payload.session.session_id().clone();
        let mut original_cwd = std::path::PathBuf::new();

        {
            // Restore model - fallback to config if not in payload (old session migration).
            let model = if payload.session.model_selection().is_no_provider() {
                self.state
                    .read()
                    .frontend
                    .app_state
                    .last_model
                    .clone()
                    .unwrap_or_default()
            } else {
                payload.session.model_selection().clone()
            };

            // Insert loaded session into HashMap.
            self.state.with_session(&self.cap, |view| {
                let session_map = view.session.map();
                session_map.insert(payload.session.clone());
            });

            // Frontend write: clear visual_parents via FrontendCap.
            {
                self.state.with_preferences(&self.frontend_cap, |ops| {
                    let frontend = ops.frontend();
                    frontend.update_sections(|s| {
                        s.sessions.visual_parents.retain(|_k, v| v != &session_id);
                    });
                });
            }

            self.state.with_session(&self.cap, |view| {
                let session_map = view.session.map();
                #[expect(clippy::expect_used, reason = "just inserted into sessions map above")]
                let session = session_map.get_mut(&session_id).expect("just inserted");
                session.set_model(model);
                // Mark loaded session as interacted - it came from disk (already persisted).
                session.mark_interacted();

                // Read cwd before releasing the lock (for async existence check).
                session.cwd().clone_into(&mut original_cwd);

                session_map.set_active(session_id.clone());
                session_map.clear_load();
            });
        }

        // Validate CWD - fallback to default if non-existent on disk.
        // This check is async (tokio::fs), so it runs outside the state lock.
        let cwd_exists = tokio::fs::try_exists(&original_cwd).await.unwrap_or(false);
        if !cwd_exists {
            let default_cwd = {
                let state = self.state.read();
                state.session.default_cwd().clone()
            };
            self.state.with_session(&self.cap, |view| {
                #[expect(clippy::expect_used, reason = "just inserted into sessions map above")]
                let session = view
                    .session
                    .map()
                    .get_mut(&session_id)
                    .expect("just inserted");
                session.push_entry(ChatEntry::system(format!(
                    "Warning: working directory '{}' not found, falling back to '{}'",
                    original_cwd.display(),
                    default_cwd.display()
                )));
                session.set_cwd(default_cwd);
            });
        }

        // Notify other actors that the active session changed.
        self.publish(ActiveSessionChanged {
            session_id: session_id.clone(),
        })
        .await;

        // Note: no scan commands are emitted here. The three scan actors
        // (skills, prompts, context-files) subscribe to the `SessionLoadCompleted`
        // event emitted below and self-trigger their per-session scans.

        // Persist the restored session.
        self.save_active_session(&session_id).await;
    }

    /// SessionForkRequested: fork the session in SQLite, then load the new session.
    ///
    /// Calls `store.fork()` to create a new session with entries up to `at_ordinal`,
    /// then uses `load_and_insert` to insert and emit `SessionLoadCompleted`,
    /// followed by the heavy restore flow.
    #[expect(clippy::expect_used, reason = "just inserted above")]
    pub(in crate::feat::session::session_actor) async fn on_session_fork_requested(
        &self,
        payload: &SessionForkRequested,
    ) {
        let store = &self.services.session_store;

        // Persist the source first: the store keeps the disk representation,
        // and `fork` copies from the stored rows. A never-saved session (no
        // rows) would fail the fork outright, and a stale snapshot would
        // silently truncate the fork to the last save. Pinning the source's
        // current state (interaction + fresh save) guarantees the fork sees
        // the full history up to the entry the user picked.
        self.state.with_session(&self.cap, |view| {
            if let Some(session) = view.session.map().get_mut(&payload.source_session_id) {
                session.mark_interacted();
            }
        });
        self.save_active_session(&payload.source_session_id).await;

        // Fork in SQLite.
        let new_id = match store
            .fork(&payload.source_session_id, payload.at_ordinal)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(err = ?e, "failed to fork session");
                // Clear loading state.
                self.state
                    .with_session(&self.cap, |view| view.session.map().clear_load());
                return;
            }
        };

        // Load the forked session.
        match store.load_session(&new_id).await {
            Ok(Some(session)) => {
                // Insert session and emit SessionLoadCompleted for external subscribers.
                self.load_and_insert(session).await;

                // Run the user-facing restore flow.
                // Re-read from state since load_and_insert consumed the session.
                let session = self
                    .state
                    .read()
                    .session
                    .get(&new_id)
                    .expect("just inserted")
                    .clone();
                let payload = SessionLoadCompleted { session };
                self.handle_session_load_completed(&payload).await;
            }
            Ok(None) => {
                tracing::warn!("forked session not found after creation");
                self.state
                    .with_session(&self.cap, |view| view.session.map().clear_load());
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load forked session");
                self.state
                    .with_session(&self.cap, |view| view.session.map().clear_load());
            }
        }
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

    use super::*;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::protocol::ChatEntry;

    #[rstest::rstest]
    #[tokio::test]
    async fn session_load_does_not_set_context_size() {
        // Given a session with chat history.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello world"));
        session.push_entry(ChatEntry::assistant("hi there"));

        let (actor, _audit) = super::super::super::helpers::test_actor_recording().await;

        let payload = SessionLoadCompleted { session };

        // When handling SessionLoadCompleted.
        actor.handle_session_load_completed(&payload).await;

        // Then context_size is NOT set by the session actor (ContextSizeActor handles this).
        let state = actor.state.read();
        let active = state.active_session();
        assert!(
            active.context_size().is_none(),
            "context_size should NOT be set by session actor (ContextSizeActor owns this)"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_session_load_completed_marks_session_as_interacted() {
        // Given a session loaded from disk.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello"));
        let session_id = session.session_id().clone();

        let (actor, _audit) = super::super::super::helpers::test_actor_recording().await;

        let payload = SessionLoadCompleted { session };

        // When handling SessionLoadCompleted.
        actor.handle_session_load_completed(&payload).await;

        // Then the session has been marked as interacted (it came from disk).
        let state = actor.state.read();
        let loaded = state.session.get(&session_id).expect("session exists");
        assert!(
            loaded.has_interacted(),
            "loaded session should be marked as interacted"
        );
        assert!(
            loaded.is_persistable(),
            "loaded session should be persistable"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_session_load_completed_emits_no_scan_commands() {
        // Given a session loaded from disk.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello"));

        let (actor, audit) = super::super::super::helpers::test_actor_recording().await;

        let payload = SessionLoadCompleted { session };

        // When handling SessionLoadCompleted.
        actor.handle_session_load_completed(&payload).await;

        // Then no scan commands are emitted. The three scan actors
        // (skills, prompts, context-files) subscribe to `SessionLoadCompleted`
        // themselves and self-trigger their per-session scans.
        assert!(
            !audit.contains_name("ScanSkills"),
            "should not emit ScanSkills"
        );
        assert!(
            !audit.contains_name("RescanPromptTemplates"),
            "should not emit RescanPromptTemplates"
        );
        assert!(
            !audit.contains_name("ScanContextFiles"),
            "should not emit ScanContextFiles"
        );
    }

    // ============================================================
    // Fork: persist-before-fork
    // ============================================================

    use crate::feat::session::protocol::session_fork_requested::SessionForkRequested;

    /// The store's persisted snapshot (what fork/load see) entry count for
    /// the session.
    fn store_entry_count(
        store: &std::sync::Arc<super::super::super::helpers::PopulatedFakeStore>,
        session_id: &crate::protocol::SessionId,
    ) -> usize {
        store
            .stored_session(session_id)
            .map_or(0, |s| s.history().len())
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fork_of_unsaved_source_persists_the_source_then_forks() {
        // Given a live session that has never been persisted (fresh, interacted,
        // with history) — and an empty store.
        let (actor, store, audit) =
            super::super::super::helpers::test_actor_with_store_recording(vec![]).await;
        let source_id = {
            let mut state = actor.state.write_test_no_cap();
            let session = state.active_session_mut();
            session.mark_interacted();
            session.push_entry(ChatEntry::user("first"));
            session.push_entry(ChatEntry::assistant("reply"));
            session.session_id().clone()
        };

        // When a fork from the last entry is requested.
        actor
            .on_session_fork_requested(&SessionForkRequested {
                source_session_id: source_id.clone(),
                at_ordinal: 1,
            })
            .await;

        // Then the source was persisted before forking (the store holds it —
        // the SQL fork would have failed with "source session not found").
        let saved_source = store.stored_session(&source_id);
        assert!(
            saved_source.is_some(),
            "fork must persist an unsaved source before forking"
        );

        // And a forked session was created, loaded, and announced.
        assert!(
            audit.contains_name("SessionLoadCompleted"),
            "fork flow must complete with a SessionLoadCompleted for the new session"
        );
        let state = actor.state.read();
        let forked_count = state
            .session
            .iter()
            .filter(|(id, s)| **id != source_id && s.parent_session().as_ref() == Some(&source_id))
            .count();
        assert_eq!(
            forked_count, 1,
            "the forked session must be inserted into the session map"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fork_after_unsaved_reply_includes_that_reply() {
        // Given a source whose stored snapshot predates its latest assistant
        // reply: the store holds one entry, the live session holds two.
        let mut stored = ChatSessionState::new();
        stored.mark_interacted();
        stored.push_entry(ChatEntry::user("question"));
        let source_id = stored.session_id().clone();
        let (actor, store, _audit) =
            super::super::super::helpers::test_actor_with_store_recording(vec![stored]).await;
        let mut live = ChatSessionState::new();
        live.set_session_id(source_id.clone());
        live.mark_interacted();
        live.push_entry(ChatEntry::user("question"));
        live.push_entry(ChatEntry::assistant("fresh answer"));
        {
            let mut state = actor.state.write_test_no_cap();
            state.session.insert(live);
            state.session.set_active(source_id.clone());
        }
        assert_eq!(
            store_entry_count(&store, &source_id),
            1,
            "precondition: the store's snapshot is stale (1 entry)"
        );

        // When a fork from the newest entry (ordinal 1) is requested.
        actor
            .on_session_fork_requested(&SessionForkRequested {
                source_session_id: source_id.clone(),
                at_ordinal: 1,
            })
            .await;

        // Then the source was re-persisted with its full history first, and
        // the forked session's history includes the fresh reply.
        let saved = store
            .stored_session(&source_id)
            .expect("source must be re-persisted before forking");
        assert_eq!(
            saved.history().len(),
            2,
            "the re-persisted source must reflect the live history"
        );

        // And the forked child (in state) carries both entries.
        let state = actor.state.read();
        let forked = state
            .session
            .iter()
            .find(|(id, s)| **id != source_id && s.parent_session().as_ref() == Some(&source_id))
            .map(|(_, s)| s)
            .expect("forked session inserted");
        assert_eq!(
            forked.history().len(),
            2,
            "the fork must include the entry it was forked from"
        );
    }
}
