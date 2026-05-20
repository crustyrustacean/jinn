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
            Ok(Some(session)) => {
                // Unarchive the session so it appears in the picker on next load.
                if let Err(e) = store.set_archived(&evt.session_id, false).await {
                    tracing::warn!(err = ?e, "failed to unarchive session on load");
                }

                let strategy_id = session.active_strategy().clone();
                let strategy_blob = session
                    .strategy_state()
                    .get(&strategy_id)
                    .and_then(|s| serde_json::to_value(s).ok())
                    .unwrap_or(serde_json::json!({}));

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
