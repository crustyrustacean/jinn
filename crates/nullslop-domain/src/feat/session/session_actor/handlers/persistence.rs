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
            let session = guard.session.sessions_mut().get_mut(&session_id)?;
            session.touch();
            Some(session.clone())
        })
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(err = ?e, "spawn_blocking panicked during session save");
            None
        });

        let Some(session) = session else { return };

        if let Err(e) = store.save(&session).await {
            tracing::warn!(
                session_id = ?session_id_log,
                err = ?e,
                "failed to persist session"
            );
        }
    }

    /// Loads a full session from disk and sends back a `SessionLoadCompleted` command.
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
                // Unarchive the session so it appears in the picker on next load.
                if let Err(e) = store.set_archived(&evt.session_id, false).await {
                    tracing::warn!(err = ?e, "failed to unarchive session on load");
                }

                // Reset in-memory state so the sidebar filter includes this session.
                session.set_session_state(
                    crate::feat::session::chat_session::SessionState::Loaded,
                );

                let _ =
                    ctx.send_command(Command::SessionLoadCompleted(CompletedPayload { session }));
            }
            Ok(None) => {
                tracing::warn!(
                    session_id = ?evt.session_id,
                    "session load returned None"
                );
                // Create an empty session with the requested ID.
                let mut session = crate::feat::session::chat_session::ChatSessionState::new();
                session.set_session_id(evt.session_id.clone());
                let _ =
                    ctx.send_command(Command::SessionLoadCompleted(CompletedPayload { session }));
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load session");
                let mut session = crate::feat::session::chat_session::ChatSessionState::new();
                session.set_session_id(evt.session_id.clone());
                let _ =
                    ctx.send_command(Command::SessionLoadCompleted(CompletedPayload { session }));
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
}
