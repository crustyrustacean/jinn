//! Session load and fork handlers - restore sessions from disk and fork session history.
//!
//! Handles restoring a loaded session into active state (validating CWD, emitting
//! follow-up commands for strategy restoration) and forking a session at a specific
//! point in its history.

use crate::common::actor::ActorContext;

use crate::feat::session::model_selection::ModelSelection;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::protocol::{ChatEntry, Event};

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
        ctx: &ActorContext,
    ) {
        let session_id = payload.session.session_id().clone();
        let original_cwd;

        {
            let mut state = self.state.write();
            let loaded = payload.session.clone();

            // Restore model - fallback to config if not in payload (old session migration).
            let model = if loaded.model_selection().is_no_provider() {
                state
                    .frontend
                    .app_state
                    .last_model
                    .clone()
                    .unwrap_or_default()
            } else {
                loaded.model_selection().clone()
            };

            // Insert loaded session into HashMap.
            state.session.insert(loaded);

            // Clear stale visual-parent entries that bypass this session.
            crate::feat::ui::sidebar::sessions::clear_visual_parents_on_load(
                &mut state,
                &session_id,
            );

            #[expect(clippy::expect_used, reason = "just inserted into sessions map above")]
            let session = state.session.get_mut(&session_id).expect("just inserted");
            session.set_model(model);
            // Mark loaded session as interacted - it came from disk (already persisted).
            session.mark_interacted();

            // Read cwd before releasing the lock (for async existence check).
            original_cwd = session.cwd().to_owned();

            state.session.set_active(session_id.clone());
            state.session.clear_load();
        }

        // Re-register attached plugins for the loaded session.
        // Must be called after the write lock above is released, since
        // rehydrate_attached_plugins acquires its own write lock.
        self.rehydrate_attached_plugins(&session_id);

        // Validate CWD - fallback to default if non-existent on disk.
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
        {
            let _ = ctx.send_event(Event::ActiveSessionChanged(
                crate::protocol::system::ActiveSessionChanged {
                    session_id: session_id.clone(),
                },
            ));
        }

        // Note: no scan commands are emitted here. The three scan actors
        // (skills, prompts, context-files) subscribe to the `SessionLoadCompleted`
        // event emitted below and self-trigger their per-session scans.

        // Persist the restored session.
        self.save_active_session(&session_id).await;
    }

    /// Reset `Running` → `Idle` for attached plugins on loaded sessions.
    ///
    /// Crash/restart safety: a plugin that was `Running` when the process died
    /// would otherwise be stuck in `Running` forever. The dispatcher will
    /// re-attach and re-fire as needed on next lifecycle event.
    ///
    /// Call this after inserting a session into the SessionMap.
    /// Acquires its own write lock — do NOT call inside another write-lock scope.
    pub(in crate::feat::session::session_actor) fn rehydrate_attached_plugins(
        &self,
        session_id: &crate::protocol::SessionId,
    ) {
        let mut state = self.state.write();
        let Some(session) = state.session.get_mut(session_id) else {
            return;
        };
        for ap in &mut session.core.attached_plugins {
            if matches!(
                ap.run_state,
                crate::feat::attached_plugin::PluginRunState::Running
            ) {
                ap.run_state = crate::feat::attached_plugin::PluginRunState::Idle;
            }
        }
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
        ctx: &ActorContext,
    ) {
        let store = &self.services.session_store;

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
                // Insert session and emit SessionLoadCompleted for external subscribers.
                self.load_and_insert(session, ctx);

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
                self.handle_session_load_completed(&payload, ctx).await;
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
    async fn session_load_does_not_set_context_size() {
        // Given a session with chat history.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello world"));
        session.push_entry(ChatEntry::assistant("hi there"));

        let actor = test_actor();
        let (_sink, ctx) = test_context();

        let payload = SessionLoadCompleted { session };

        // When handling SessionLoadCompleted.
        actor.handle_session_load_completed(&payload, &ctx).await;

        // Then context_size is NOT set by the session actor (ContextSizeActor handles this).
        let state = actor.state.read();
        let active = state.active_session();
        assert!(
            active.context_size().is_none(),
            "context_size should NOT be set by session actor (ContextSizeActor owns this)"
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
        actor.handle_session_load_completed(&payload, &ctx).await;

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

    #[tokio::test]
    async fn handle_session_load_completed_emits_no_scan_commands() {
        // Given a session loaded from disk.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello"));

        let actor = test_actor();
        let (sink, ctx) = test_context();

        let payload = SessionLoadCompleted { session };

        // When handling SessionLoadCompleted.
        actor.handle_session_load_completed(&payload, &ctx).await;

        // Then no scan commands are emitted. The three scan actors
        // (skills, prompts, context-files) subscribe to `SessionLoadCompleted`
        // themselves and self-trigger their per-session scans.
        let scan_commands = sink
            .commands()
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    crate::protocol::Command::ScanSkills(_)
                        | crate::protocol::Command::RescanPromptTemplates(_)
                        | crate::protocol::Command::ScanContextFiles(_)
                )
            })
            .count();
        assert_eq!(
            scan_commands, 0,
            "scan actors self-trigger off SessionLoadCompleted; load handler should not emit scan commands"
        );
    }
}
