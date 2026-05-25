//! Session load and fork handlers — restore sessions from disk and fork session history.
//!
//! Handles restoring a loaded session into active state (validating CWD, emitting
//! follow-up commands for strategy restoration) and forking a session at a specific
//! point in its history.

use crate::common::actor::ActorContext;
use crate::feat::context::assemble::assemble_prompt;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::protocol::{ChatEntry, Command, Event};

use super::super::SessionPersistenceActor;
use crate::SessionForkRequested;

impl SessionPersistenceActor {
    /// SessionLoadCompleted: restore session state and emit follow-up commands.
    pub(in crate::feat::session::session_actor) async fn handle_session_load_completed(
        &self,
        payload: &SessionLoadCompleted,
        ctx: &ActorContext,
    ) {
        let session_id = payload.session.session_id().clone();
        let original_cwd;

        {
            let mut state = self.state.write();
            let loaded = payload.session.clone();

            // Restore model — fallback to config if not in payload (old session migration).
            let model = if loaded.model() == crate::feat::provider_infra::NO_PROVIDER_ID {
                state
                    .frontend
                    .preferences
                    .last_model
                    .clone()
                    .unwrap_or_else(|| crate::feat::provider_infra::NO_PROVIDER_ID.to_owned())
            } else {
                loaded.model().to_owned()
            };

            let title_text = loaded.title().unwrap_or("Untitled Session").to_owned();

            // Insert loaded session into HashMap.
            state.session.insert(loaded);

            // Clear stale visual-parent entries that bypass this session.
            crate::feat::ui::sidebar::sessions::clear_visual_parents_on_load(
                &mut state,
                &session_id,
            );

            // Add a system message about the restore.
            #[expect(clippy::expect_used, reason = "just inserted into sessions map above")]
            let session = state.session.get_mut(&session_id).expect("just inserted");
            session.push_entry(ChatEntry::system(format!("Session restored: {title_text}")));
            session.set_model(model);
            // Mark loaded session as interacted — it came from disk (already persisted).
            session.mark_interacted();

            // Read cwd before releasing the lock (for async existence check).
            original_cwd = session.cwd().to_owned();

            state.session.set_active(session_id.clone());
            state.session.clear_load();
        }

        // Validate CWD — fallback to default if non-existent on disk.
        // This check is async (tokio::fs), so it runs outside the state lock.
        let cwd_exists = tokio::fs::try_exists(&original_cwd).await.unwrap_or(false);
        if !cwd_exists {
            let default_cwd = {
                let state = self.state.read();
                state.session.default_cwd().clone()
            };
            let mut state = self.state.write();
            #[expect(clippy::expect_used, reason = "just inserted into sessions map above")]
            let session = state.session.get_mut(&session_id).expect("just inserted");
            session.push_entry(ChatEntry::system(format!(
                "Warning: working directory '{}' not found, falling back to '{}'",
                original_cwd.display(),
                default_cwd.display()
            )));
            session.set_cwd(default_cwd);
        }

        // Notify other actors that the active session changed.
        // (Moved outside the first lock block since we restructured.)
        {
            let _ = ctx.send_event(Event::ActiveSessionChanged(
                crate::protocol::system::ActiveSessionChanged {
                    session_id: session_id.clone(),
                },
            ));
        }

        // Recalculate context size for the status bar.
        // cached_context_size is ephemeral (not persisted), so it's None after load.
        // Running assemble_prompt once gives an accurate current context size.
        let assembled = {
            let guard = self.state.read();
            assemble_prompt(&guard, &session_id, &self.counter, None)
        };
        {
            let mut state = self.state.write();
            let session = state.session.get_mut(&session_id).expect("just inserted");
            session.set_context_size(assembled.estimated_tokens());
        }

        // Persist the restored session (includes the "Session restored" system entries).
        self.save_active_session(&session_id).await;
    }

    /// SessionForkRequested: fork the session in SQLite, then load the new session.
    ///
    /// Calls `store.fork()` to create a new session with entries up to `at_ordinal`,
    /// then emits `SessionLoadCompleted` to trigger the standard session-load flow.
    pub(in crate::feat::session::session_actor) async fn on_session_fork_requested(
        &self,
        payload: &SessionForkRequested,
        ctx: &ActorContext,
    ) {
        let Some(store) = &self.store else {
            tracing::warn!("session-actor has no store — dropping fork request");
            return;
        };

        // Fork in SQLite.
        let new_id = match store
            .fork(&payload.source_session_id, payload.at_ordinal)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(err = ?e, "failed to fork session");
                // Clear loading state.
                let mut state = self.state.write();
                state.session.clear_load();
                return;
            }
        };

        // Load the forked session.
        match store.load_session(&new_id).await {
            Ok(Some(session)) => {
                if let Err(e) = ctx.send_command(Command::SessionLoadCompleted(
                    crate::feat::session::protocol::session_load_completed::SessionLoadCompleted {
                        session,
                    },
                )) {
                    tracing::warn!(err = ?e, "session-actor failed to emit SessionLoadCompleted after fork");
                }
            }
            Ok(None) => {
                tracing::warn!("forked session not found after creation");
                let mut state = self.state.write();
                state.session.clear_load();
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load forked session");
                let mut state = self.state.write();
                state.session.clear_load();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::protocol::ChatEntry;

    fn test_actor() -> SessionPersistenceActor {
        super::super::super::helpers::test_actor()
    }

    fn test_context() -> (
        std::sync::Arc<crate::common::actor::RecordingSink>,
        crate::common::actor::ActorContext,
    ) {
        super::super::super::helpers::test_context()
    }

    #[tokio::test]
    async fn session_load_populates_context_size() {
        // Given a session with chat history.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello world"));
        session.push_entry(ChatEntry::assistant("hi there"));

        let actor = test_actor();
        let (_sink, ctx) = test_context();

        let payload = SessionLoadCompleted { session };

        // When handling SessionLoadCompleted.
        actor.handle_session_load_completed(&payload, &ctx).await;

        // Then context_size is populated (not None).
        let state = actor.state.read();
        let active = state.active_session();
        assert!(
            active.context_size().is_some(),
            "context_size should be populated after session load"
        );
        assert!(
            active.context_size().unwrap() > 0,
            "context_size should be positive"
        );
    }

    #[tokio::test]
    async fn handle_session_load_completed_marks_session_as_interacted() {
        // Given a session loaded from disk.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello"));
        let session_id = session.session_id().clone();

        let actor = test_actor();
        let (_sink, ctx) = test_context();

        let payload = SessionLoadCompleted { session };

        // When handling SessionLoadCompleted.
        actor
            .handle_session_load_completed(&payload, &ctx)
            .await;

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
}
