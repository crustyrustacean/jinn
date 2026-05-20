//! Command handlers — process session lifecycle commands.

use crate::SessionForkRequested;
use crate::common::actor::ActorContext;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::compaction_actor::protocol::command::{BeginCompaction, EndCompaction};
use crate::feat::context::protocol::command::{
    AssemblePrompt, RestoreStrategyState, SwitchPromptStrategy,
};
use crate::feat::provider::protocol::command::SendMessage;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::protocol::{ChatEntry, ChatEntryId, ChatEntryKind, Command, Event};

use super::super::SessionPersistenceActor;
use crate::feat::session::chat_session::SessionPhase;

/// Decision returned after inspecting session state in `EnqueueUserMessage`.
enum EnqueueAction {
    /// Session is idle — dispatch prompt assembly.
    AssemblePrompt,
    /// Session is busy — message was queued.
    Queued,
}

impl SessionPersistenceActor {
    /// EnqueueUserMessage: if idle → assemble prompt; if busy → queue.
    pub(in crate::feat::session::session_actor) async fn handle_enqueue_user_message(
        &self,
        payload: &EnqueueUserMessage,
        ctx: &ActorContext,
    ) {
        let action = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            match session.phase() {
                SessionPhase::Idle => {
                    // Set title on first user message.
                    if session.title().is_none() {
                        let title = match &payload.entry.kind {
                            ChatEntryKind::User { display, .. } => {
                                display.lines().next().unwrap_or("").to_owned()
                            }
                            _ => String::new(),
                        };
                        session.set_title(title);
                    }
                    session.push_entry(payload.entry.clone());
                    session.begin_sending();
                    EnqueueAction::AssemblePrompt
                }
                SessionPhase::Sending
                | SessionPhase::Streaming
                | SessionPhase::Assembling
                | SessionPhase::Compacting => {
                    session.enqueue_message(payload.entry.clone());
                    EnqueueAction::Queued
                }
            }
        };

        let (history, model_name) = match action {
            EnqueueAction::AssemblePrompt => {
                let state = self.state.read();
                let history = state.session(&payload.session_id).history().to_vec();
                let model_name = state.session(&payload.session_id).profile().model.clone();
                (history, model_name)
            }
            EnqueueAction::Queued => (vec![], String::new()),
        };

        match action {
            EnqueueAction::AssemblePrompt => {
                if let Err(e) = ctx.send_command(Command::AssemblePrompt(AssemblePrompt {
                    session_id: payload.session_id.clone(),
                    history,
                    tools: vec![],
                    model_name,
                })) {
                    tracing::warn!(err = ?e, "session-actor failed to emit AssemblePrompt");
                }

                if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
                    session_id: payload.session_id.clone(),
                    entry: payload.entry.clone(),
                })) {
                    tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
                }

                self.save_active_session(&payload.session_id).await;
            }
            EnqueueAction::Queued => {}
        }
    }

    /// SetChatInputText: update the session's input buffer.
    pub(in crate::feat::session::session_actor) fn handle_set_chat_input_text(
        &self,
        payload: &SetChatInputText,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.chat_input_mut().replace_all(payload.text.clone());
    }

    /// PushChatEntry: push entry to session history, emit ChatEntrySubmitted event,
    /// and persist the session to disk.
    pub(in crate::feat::session::session_actor) async fn handle_push_chat_entry(
        &self,
        payload: &PushChatEntry,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.push_entry(payload.entry.clone());
        }

        if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
            session_id: payload.session_id.clone(),
            entry: payload.entry.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
        }

        self.save_active_session(&payload.session_id).await;
    }

    /// SendMessage: backward compat — emit EnqueueUserMessage.
    pub(in crate::feat::session::session_actor) fn handle_send_message(
        payload: &SendMessage,
        ctx: &ActorContext,
    ) {
        if let Err(e) = ctx.send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
            session_id: payload.session_id.clone(),
            entry: ChatEntry::user(&payload.text),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit EnqueueUserMessage");
        }
    }

    /// SessionLoadCompleted: restore session state and emit follow-up commands.
    pub(in crate::feat::session::session_actor) async fn handle_session_load_completed(
        &self,
        payload: &SessionLoadCompleted,
        ctx: &ActorContext,
    ) {
        let session_id = payload.session.session_id().clone();
        let strategy_id = payload.session.active_strategy().clone();
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
            state
                .session
                .sessions_mut()
                .insert(session_id.clone(), loaded);

            // Add a system message about the restore.
            #[expect(clippy::expect_used, reason = "just inserted into sessions map above")]
            let session = state
                .session
                .sessions_mut()
                .get_mut(&session_id)
                .expect("just inserted");
            session.push_entry(ChatEntry::system(format!("Session restored: {title_text}")));
            session.set_model(model);

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
            let session = state
                .session
                .sessions_mut()
                .get_mut(&session_id)
                .expect("just inserted");
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

        // Serialize the strategy state for RestoreStrategyState.
        let blob = {
            let state = self.state.read();
            #[expect(clippy::expect_used, reason = "just inserted into sessions map above")]
            let session = state
                .session
                .sessions()
                .get(&session_id)
                .expect("just inserted");
            session
                .strategy_state()
                .get(&strategy_id)
                .and_then(|s| serde_json::to_value(s).ok())
                .unwrap_or(serde_json::json!({}))
        };

        if let Err(e) = ctx.send_command(Command::RestoreStrategyState(RestoreStrategyState {
            session_id: session_id.clone(),
            strategy_id: strategy_id.clone(),
            blob,
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit RestoreStrategyState");
        }

        if let Err(e) = ctx.send_command(Command::SwitchPromptStrategy(SwitchPromptStrategy {
            session_id: session_id.clone(),
            strategy_id,
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SwitchPromptStrategy");
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

    /// BeginCompaction: set phase to Compacting, push "Starting..." system entry,
    /// mark gathered entries as ignored, and persist.
    pub(in crate::feat::session::session_actor) async fn handle_begin_compaction(
        &self,
        payload: &BeginCompaction,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.begin_compacting(payload.gathered_indices.clone());
            session.push_entry(ChatEntry::system("Starting context compaction..."));
            if !payload.gathered_indices.is_empty() {
                session.mark_entries_ignored(&payload.gathered_indices);
            }
        }

        self.save_active_session(&payload.session_id).await;
    }

    /// EndCompaction: insert compaction entry or error entry, set phase to Idle,
    /// drain any queued messages, persist, and start a new turn if needed.
    ///
    /// Ignores the payload if the session is not currently in Compacting phase
    /// (e.g. compaction was cancelled while the LLM call was in flight).
    pub(in crate::feat::session::session_actor) async fn handle_end_compaction(
        &self,
        payload: &EndCompaction,
        ctx: &ActorContext,
    ) {
        let drained_entries: Vec<ChatEntry>;
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);

            // Guard: ignore stale EndCompaction if phase is no longer Compacting.
            if !matches!(session.phase(), SessionPhase::Compacting) {
                tracing::warn!(
                    session_id = ?payload.session_id,
                    current_phase = ?session.phase(),
                    "EndCompaction received but session is not compacting — ignoring"
                );
                return;
            }

            if let Some(result) = &payload.result {
                let compaction_entry = ChatEntry {
                    id: ChatEntryId::new(),
                    timestamp: jiff::Timestamp::now(),
                    kind: ChatEntryKind::Compaction {
                        summary: result.summary.clone(),
                        tokens_before: result.tokens_before,
                        entries_compacted: result.entries_compacted,
                        model_used: result.model_used.clone(),
                    },
                    pin_position: None,
                    ignored: false,
                };
                session.insert_entry_at(result.boundary_index, compaction_entry);
                session.push_entry(ChatEntry::system(format!(
                    "Context was compacted. {} messages were summarized.",
                    result.entries_compacted
                )));
            } else {
                let error_msg = payload.error.as_deref().unwrap_or("Unknown error");
                session.push_entry(ChatEntry::error(format!("Compaction failed: {error_msg}")));
            }
            session.finish_compacting();

            // Drain any messages queued during compaction.
            drained_entries = session.drain_queue().into_iter().collect();
        }

        self.save_active_session(&payload.session_id).await;

        // If messages were queued during compaction, start a new turn.
        if !drained_entries.is_empty() {
            self.start_turn_from_queued(&payload.session_id, &drained_entries, ctx)
                .await;
        }
    }
}
