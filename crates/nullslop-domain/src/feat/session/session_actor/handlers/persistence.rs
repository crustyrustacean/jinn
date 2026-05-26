//! Persistence handlers — save and load session snapshots.

use super::super::SessionPersistenceActor;
use crate::SessionLoadRequested;
use crate::protocol::Command;

impl SessionPersistenceActor {
    /// Saves the current state of a session to disk.
    ///
    /// Clones the session inside `spawn_blocking` to avoid blocking the
    /// async runtime with a potentially expensive `ChatSessionState` clone
    /// (which includes the full `Vec<ChatEntry>` history). The store's
    /// `save` method does its own `spawn_blocking` internally for SQLite I/O.
    /// Errors are logged as warnings — persistence failure must not break
    /// the user experience.
    pub(in crate::feat::session::session_actor) async fn save_active_session(
        &self,
        session_id: &crate::protocol::SessionId,
    ) {
        let Some(store) = &self.store else {
            tracing::warn!("session-actor has no store — skipping save");
            return;
        };

        let state = self.state.clone();
        let session_id = session_id.clone();
        let session_id_log = session_id.clone();

        let session = tokio::task::spawn_blocking(move || {
            let mut guard = state.write();
            let session = guard.session.get_mut(&session_id)?;
            session.touch();
            Some(session.clone())
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
        ctx: &crate::common::actor::ActorContext,
    ) {
        {
            let mut state = self.state.write();
            if let Some(session) = state.session.get_mut(&payload.session_id) {
                session.mark_interacted();
            }
        }

        if let Err(e) = ctx.send_event(crate::protocol::Event::UserInteracted(
            crate::feat::session::protocol::user_interacted::UserInteracted {
                session_id: payload.session_id.clone(),
            },
        )) {
            tracing::warn!(err = ?e, "session-actor failed to emit UserInteracted");
        }

        self.save_active_session(&payload.session_id).await;
    }

    /// Creates an empty session with the given ID and emits a `SessionLoadCompleted` command.
    ///
    /// Used as a fallback when a session is not found or fails to load.
    fn create_empty_session_response(
        &self,
        session_id: &crate::protocol::SessionId,
        ctx: &crate::common::actor::ActorContext,
    ) {
        use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted as CompletedPayload;

        let mut session = crate::feat::session::chat_session::ChatSessionState::new();
        session.set_session_id(session_id.clone());
        let _ = ctx.send_command(Command::SessionLoadCompleted(CompletedPayload { session }));
    }

    /// Loads all judge sessions belonging to the given origin from the store,
    /// unarchives them, sets their state to `Loaded`, and inserts them into
    /// the in-memory session map.
    async fn load_and_insert_judge_sessions(
        &self,
        store: &crate::feat::session::SessionStoreService,
        origin_id: &crate::protocol::SessionId,
    ) {
        let judge_sessions =
            match store.load_judge_sessions_for_origin(origin_id).await {
                Ok(sessions) => sessions,
                Err(e) => {
                    tracing::warn!(
                        err = ?e,
                        "failed to load judge sessions for origin"
                    );
                    Vec::new()
                }
            };

        if !judge_sessions.is_empty() {
            for judge_session in &judge_sessions {
                let judge_id = judge_session.session_id().clone();
                if let Err(e) = store.set_archived(&judge_id, false).await {
                    tracing::warn!(
                        err = ?e,
                        judge_session_id = %judge_id,
                        "failed to unarchive judge session"
                    );
                }
            }
            let mut state = self.state.write();
            for mut judge_session in judge_sessions {
                judge_session.set_session_state(
                    crate::feat::session::chat_session::SessionState::Loaded,
                );
                state.session.insert(judge_session);
            }
        }
    }

    /// Loads a full session from disk and sends back a `SessionLoadCompleted` command.
    ///
    /// If the requested session is a judge, redirects to loading its origin session
    /// instead. The judge gets loaded as a side-effect of the origin's auto-load.
    pub(in crate::feat::session::session_actor) async fn on_load_requested(
        &mut self,
        evt: &SessionLoadRequested,
        ctx: &crate::common::actor::ActorContext,
    ) {
        use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted as CompletedPayload;

        let Some(store) = &self.store else {
            tracing::warn!("session-actor has no store — dropping load request");
            return;
        };

        match store.load_session(&evt.session_id).await {
            Ok(Some(mut session)) => {
                // --- Judge-to-origin redirect ---
                // If the loaded session is a judge, load its origin instead.
                // The judge will be auto-loaded as a side-effect of loading the origin.
                if let Some(meta) = session.judge() {
                    let origin_id = meta.origin_session.clone();

                    // Guard: self-referential judge — proceed as normal session.
                    if origin_id == evt.session_id {
                        tracing::warn!(
                            session_id = %evt.session_id,
                            "judge session references itself as origin, loading normally"
                        );
                    } else {
                        tracing::info!(
                            judge_session = %evt.session_id,
                            origin_session = %origin_id,
                            "requested session is a judge, redirecting to origin"
                        );

                        match store.load_session(&origin_id).await {
                            Ok(Some(mut origin_session)) => {
                                // Unarchive + reset the origin.
                                if let Err(e) = store.set_archived(&origin_id, false).await {
                                    tracing::warn!(err = ?e, "failed to unarchive origin session");
                                }
                                origin_session.set_session_state(
                                    crate::feat::session::chat_session::SessionState::Loaded,
                                );

                                // Swap the load guard from judge to origin.
                                {
                                    let mut state = self.state.write();
                                    state.session.clear_load();
                                    state.session.begin_load(origin_id.clone());
                                }

                                // Auto-load judge sessions for the origin.
                                self.load_and_insert_judge_sessions(store, &origin_id).await;

                                let _ = ctx.send_command(Command::SessionLoadCompleted(
                                    CompletedPayload {
                                        session: origin_session,
                                    },
                                ));
                                return;
                            }
                            Ok(None) => {
                                tracing::warn!(
                                    origin_session_id = %origin_id,
                                    "origin session not found, falling back to judge"
                                );
                                // Fall through — load the judge as the active session.
                            }
                            Err(e) => {
                                tracing::warn!(
                                    err = ?e,
                                    origin_session_id = %origin_id,
                                    "failed to load origin session, falling back to judge"
                                );
                                // Fall through — load the judge as the active session.
                            }
                        }
                    }
                }

                // Unarchive the session so it appears in the picker on next load.
                if let Err(e) = store.set_archived(&evt.session_id, false).await {
                    tracing::warn!(err = ?e, "failed to unarchive session on load");
                }

                // Reset in-memory state so the sidebar filter includes this session.
                session.set_session_state(
                    crate::feat::session::chat_session::SessionState::Loaded,
                );

                // Load judge sessions that belong to this origin.
                // They may have been cascade-archived alongside the origin.
                // We unarchive and insert them into memory so the coordinator
                // can trigger them on the next IDLE transition.
                self.load_and_insert_judge_sessions(store, &evt.session_id).await;

                let _ =
                    ctx.send_command(Command::SessionLoadCompleted(CompletedPayload { session }));
            }
            Ok(None) => {
                tracing::warn!(
                    session_id = ?evt.session_id,
                    "session load returned None"
                );
                self.create_empty_session_response(&evt.session_id, ctx);
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load session");
                self.create_empty_session_response(&evt.session_id, ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::super::super::helpers::{test_actor_with_store, test_context};
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::session::chat_session::SessionState;
    use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
    use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
    use crate::protocol::Command;

    #[tokio::test]
    async fn loading_archived_session_resets_state_to_loaded() {
        // Given an archived session in the store.
        let mut store_session = ChatSessionState::new();
        store_session.set_title("Archived Chat".to_owned());
        store_session.set_session_state(SessionState::Archived);
        let session_id = store_session.session_id().clone();
        let (mut actor, _store) = test_actor_with_store(vec![store_session]);
        let (sink, ctx) = test_context();

        // When loading the archived session.
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then SessionLoadCompleted is emitted with session_state == Loaded.
        let loaded_session = sink
            .commands()
            .iter()
            .find_map(|cmd| match cmd {
                Command::SessionLoadCompleted(payload) => Some(payload.session.clone()),
                _ => None,
            })
            .expect("expected SessionLoadCompleted command");

        assert_eq!(
            loaded_session.session_state(),
            SessionState::Loaded,
            "loaded session should have SessionState::Loaded"
        );

        // And the session appears in sorted_open_sessions.
        let mut state = actor.state.write();
        state
            .session
            .sessions_mut()
            .insert(session_id.clone(), loaded_session);
        state.session.set_active(session_id.clone());

        let sidebar_sessions = sorted_open_sessions(&state);
        assert!(
            sidebar_sessions.iter().any(|s| s.id == session_id),
            "archived session should appear in sidebar after loading"
        );
    }

    #[tokio::test]
    async fn save_active_session_skips_non_persistable_session() {
        // Given an actor with a new (non-interacted) session and a recording store.
        let (actor, store) = test_actor_with_store(vec![]);
        let session_id = actor.state.read().session.active_session_id().clone();

        // When saving the active session.
        actor.save_active_session(&session_id).await;

        // Then the session was NOT saved because it is not persistable.
        assert!(
            store.last_saved_session(&session_id).is_none(),
            "non-interacted session should not be persisted"
        );
    }

    #[tokio::test]
    async fn save_active_session_persists_interacted_session() {
        // Given an actor with an interacted session and a recording store.
        let (actor, store) = test_actor_with_store(vec![]);
        let session_id = actor.state.read().session.active_session_id().clone();
        {
            let mut state = actor.state.write();
            state.active_session_mut().mark_interacted();
        }

        // When saving the active session.
        actor.save_active_session(&session_id).await;

        // Then the session was saved.
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "interacted session should be persisted"
        );
    }

    #[tokio::test]
    async fn handle_mark_session_interacted_sets_flag_emits_event_and_persists() {
        use crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted;
        use crate::protocol::Event;

        // Given an actor with a new session.
        let (mut actor, store) = test_actor_with_store(vec![]);
        let session_id = actor.state.read().session.active_session_id().clone();
        let (sink, ctx) = test_context();

        // When handling MarkSessionInteracted.
        actor.handle_mark_session_interacted(
            &MarkSessionInteracted {
                session_id: session_id.clone(),
            },
            &ctx,
        ).await;

        // Then the session has_interacted flag is set.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(session.has_interacted());
        assert!(session.is_persistable());

        // And a UserInteracted event was emitted.
        let has_event = sink.events().iter().any(|e| {
            matches!(e, Event::UserInteracted(e) if e.session_id == session_id)
        });
        assert!(has_event, "UserInteracted event should be emitted");

        // And the session was persisted to the store.
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "interacted session should be persisted after MarkSessionInteracted"
        );
    }

    #[tokio::test]
    async fn loading_origin_auto_loads_archived_judge_sessions() {
        use crate::feat::judge::JudgeMeta;

        // Given an origin session and two archived judge sessions in the store.
        let mut origin = ChatSessionState::new();
        origin.set_title("Origin Chat".to_owned());
        origin.mark_interacted();
        let origin_id = origin.session_id().clone();

        let mut judge_a = ChatSessionState::new();
        judge_a.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "judge-a".to_owned(),
        });
        judge_a.set_session_state(SessionState::Archived);
        judge_a.mark_interacted();
        let judge_a_id = judge_a.session_id().clone();

        let mut judge_b = ChatSessionState::new();
        judge_b.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "judge-b".to_owned(),
        });
        judge_b.set_session_state(SessionState::Archived);
        judge_b.mark_interacted();
        let judge_b_id = judge_b.session_id().clone();

        let (mut actor, _store) =
            test_actor_with_store(vec![origin, judge_a, judge_b]);
        let (sink, ctx) = test_context();

        // When loading the origin session.
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: origin_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then SessionLoadCompleted is emitted with the origin session.
        let loaded_session = sink
            .commands()
            .iter()
            .find_map(|cmd| match cmd {
                Command::SessionLoadCompleted(payload) => Some(payload.session.clone()),
                _ => None,
            })
            .expect("expected SessionLoadCompleted command");
        assert_eq!(
            loaded_session.session_id(),
            &origin_id,
            "should emit origin session, not a judge"
        );

        // And both judge sessions are in the session map.
        let state = actor.state.read();
        assert!(
            state.session.contains(&judge_a_id),
            "judge_a should be in session map"
        );
        assert!(
            state.session.contains(&judge_b_id),
            "judge_b should be in session map"
        );
    }

    #[tokio::test]
    async fn judge_sessions_reset_to_loaded_state_on_origin_load() {
        use crate::feat::judge::JudgeMeta;

        // Given an origin and an archived judge session.
        let mut origin = ChatSessionState::new();
        origin.mark_interacted();
        let origin_id = origin.session_id().clone();

        let mut judge = ChatSessionState::new();
        judge.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "test-judge".to_owned(),
        });
        judge.set_session_state(SessionState::Archived);
        judge.mark_interacted();
        let judge_id = judge.session_id().clone();

        let (mut actor, _store) = test_actor_with_store(vec![origin, judge]);
        let (_sink, ctx) = test_context();

        // When loading the origin.
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: origin_id,
                },
                &ctx,
            )
            .await;

        // Then the judge session in the map has Loaded state.
        let state = actor.state.read();
        let loaded_judge = state.session.get(&judge_id).expect("judge in map");
        assert_eq!(
            loaded_judge.session_state(),
            SessionState::Loaded,
            "judge should be reset to Loaded state"
        );
    }

    #[tokio::test]
    async fn loading_origin_with_no_judges_works_normally() {
        // Given an origin session with no judge sessions.
        let mut origin = ChatSessionState::new();
        origin.set_title("Lonely Chat".to_owned());
        origin.mark_interacted();
        let origin_id = origin.session_id().clone();

        let (mut actor, _store) = test_actor_with_store(vec![origin]);
        let (sink, ctx) = test_context();

        // When loading the origin.
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: origin_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then SessionLoadCompleted is emitted.
        let loaded = sink
            .commands()
            .iter()
            .find_map(|cmd| match cmd {
                Command::SessionLoadCompleted(payload) => Some(payload.session.clone()),
                _ => None,
            })
            .expect("expected SessionLoadCompleted");
        assert_eq!(loaded.session_id(), &origin_id);
    }

    #[tokio::test]
    async fn loading_judge_session_redirects_to_origin() {
        use crate::feat::judge::JudgeMeta;

        // Given an origin and a judge session.
        let mut origin = ChatSessionState::new();
        origin.set_title("Origin Chat".to_owned());
        origin.mark_interacted();
        let origin_id = origin.session_id().clone();

        let mut judge = ChatSessionState::new();
        judge.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "test-judge".to_owned(),
        });
        judge.set_session_state(SessionState::Archived);
        judge.mark_interacted();
        let judge_id = judge.session_id().clone();

        let (mut actor, _store) = test_actor_with_store(vec![origin, judge]);
        let (sink, ctx) = test_context();

        // When loading the judge session (by its ID).
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: judge_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then SessionLoadCompleted emits the origin session, not the judge.
        let loaded = sink
            .commands()
            .iter()
            .find_map(|cmd| match cmd {
                Command::SessionLoadCompleted(payload) => Some(payload.session.clone()),
                _ => None,
            })
            .expect("expected SessionLoadCompleted");
        assert_eq!(
            loaded.session_id(),
            &origin_id,
            "should emit origin session, not the judge"
        );

        // And the judge is in the session map.
        let state = actor.state.read();
        assert!(
            state.session.contains(&judge_id),
            "judge should be in session map"
        );
    }

    #[tokio::test]
    async fn loading_judge_session_auto_loads_all_siblings() {
        use crate::feat::judge::JudgeMeta;

        // Given an origin and two judge sessions.
        let mut origin = ChatSessionState::new();
        origin.mark_interacted();
        let origin_id = origin.session_id().clone();

        let mut judge_a = ChatSessionState::new();
        judge_a.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "judge-a".to_owned(),
        });
        judge_a.set_session_state(SessionState::Archived);
        judge_a.mark_interacted();
        let judge_a_id = judge_a.session_id().clone();

        let mut judge_b = ChatSessionState::new();
        judge_b.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "judge-b".to_owned(),
        });
        judge_b.set_session_state(SessionState::Archived);
        judge_b.mark_interacted();
        let judge_b_id = judge_b.session_id().clone();

        let (mut actor, _store) =
            test_actor_with_store(vec![origin, judge_a, judge_b]);
        let (_sink, ctx) = test_context();

        // When loading judge_a (not the origin).
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: judge_a_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then both judges are in the session map.
        let state = actor.state.read();
        assert!(
            state.session.contains(&judge_a_id),
            "judge_a should be in session map"
        );
        assert!(
            state.session.contains(&judge_b_id),
            "judge_b should be in session map"
        );
    }

    #[tokio::test]
    async fn loading_judge_with_missing_origin_falls_back_to_judge() {
        use crate::feat::judge::JudgeMeta;

        // Given a judge session whose origin does not exist in the store.
        let mut judge = ChatSessionState::new();
        judge.set_judge(JudgeMeta {
            origin_session: "nonexistent-session-id".to_string().into(),
            is_attached: true,
            judge_name: "orphan-judge".to_owned(),
        });
        judge.mark_interacted();
        let judge_id = judge.session_id().clone();

        let (mut actor, _store) = test_actor_with_store(vec![judge]);
        let (sink, ctx) = test_context();

        // When loading the judge session.
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: judge_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then SessionLoadCompleted emits the judge (fallback).
        let loaded = sink
            .commands()
            .iter()
            .find_map(|cmd| match cmd {
                Command::SessionLoadCompleted(payload) => Some(payload.session.clone()),
                _ => None,
            })
            .expect("expected SessionLoadCompleted");
        assert_eq!(
            loaded.session_id(),
            &judge_id,
            "should fall back to judge when origin is missing"
        );
    }

    #[tokio::test]
    async fn loading_judge_whose_origin_already_in_memory() {
        use crate::feat::judge::JudgeMeta;

        // Given an origin already in the session map and a judge in the store.
        let mut origin = ChatSessionState::new();
        origin.set_title("In-Memory Origin".to_owned());
        origin.mark_interacted();
        let origin_id = origin.session_id().clone();

        let mut judge = ChatSessionState::new();
        judge.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "test-judge".to_owned(),
        });
        judge.set_session_state(SessionState::Archived);
        judge.mark_interacted();
        let judge_id = judge.session_id().clone();

        // The origin is loaded from the store (it's not in memory before this call).
        let (mut actor, _store) = test_actor_with_store(vec![origin, judge]);
        let (sink, ctx) = test_context();

        // When loading the judge.
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: judge_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the origin is the emitted session.
        let loaded = sink
            .commands()
            .iter()
            .find_map(|cmd| match cmd {
                Command::SessionLoadCompleted(payload) => Some(payload.session.clone()),
                _ => None,
            })
            .expect("expected SessionLoadCompleted");
        assert_eq!(
            loaded.session_id(),
            &origin_id,
            "should emit origin session"
        );
    }

    #[tokio::test]
    async fn self_referential_judge_loads_normally() {
        use crate::feat::judge::JudgeMeta;

        // Given a session that is a judge referencing itself.
        let mut session = ChatSessionState::new();
        session.mark_interacted();
        let session_id = session.session_id().clone();
        session.set_judge(JudgeMeta {
            origin_session: session_id.clone(),
            is_attached: true,
            judge_name: "self-ref".to_owned(),
        });

        let (mut actor, _store) = test_actor_with_store(vec![session]);
        let (sink, ctx) = test_context();

        // When loading this session.
        actor
            .on_load_requested(
                &SessionLoadRequested {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then SessionLoadCompleted emits it normally (no infinite loop).
        let loaded = sink
            .commands()
            .iter()
            .find_map(|cmd| match cmd {
                Command::SessionLoadCompleted(payload) => Some(payload.session.clone()),
                _ => None,
            })
            .expect("expected SessionLoadCompleted");
        assert_eq!(
            loaded.session_id(),
            &session_id,
            "self-referential judge should load normally"
        );
    }
}
