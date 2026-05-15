//! Command handlers — process session lifecycle commands.

use crate::common::actor::ActorContext;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::context::protocol::command::{
    AssemblePrompt, RestoreStrategyState, SwitchPromptStrategy,
};
use crate::feat::provider::protocol::command::SendMessage;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::protocol::{ChatEntry, Command, Event};

use super::super::SessionPersistenceActor;

/// Decision returned after inspecting session state in `EnqueueUserMessage`.
enum EnqueueAction {
    /// Session is idle — dispatch prompt assembly.
    AssemblePrompt,
    /// Session is busy — message was queued.
    Queued,
}

impl SessionPersistenceActor {
    /// EnqueueUserMessage: if idle → assemble prompt; if busy → queue.
    pub(in crate::feat::session::session_actor) fn handle_enqueue_user_message(
        &self,
        payload: &EnqueueUserMessage,
        ctx: &ActorContext,
    ) {
        let action = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            if session.is_idle() {
                // Set title on first user message.
                if session.title().is_none() {
                    let title = payload.text.lines().next().unwrap_or("").to_owned();
                    session.set_title(title);
                }
                session.push_entry(ChatEntry::user(&payload.text));
                session.begin_sending();
                EnqueueAction::AssemblePrompt
            } else {
                // All busy states (sending, streaming, assembling) → queue.
                session.enqueue_message(payload.text.clone());
                EnqueueAction::Queued
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
                    entry: ChatEntry::user(&payload.text),
                })) {
                    tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
                }

                self.save_active_session(&payload.session_id);
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

    /// PushChatEntry: push entry to session history, emit ChatEntrySubmitted event.
    pub(in crate::feat::session::session_actor) fn handle_push_chat_entry(
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
    }

    /// SendMessage: backward compat — emit EnqueueUserMessage.
    pub(in crate::feat::session::session_actor) fn handle_send_message(
        payload: &SendMessage,
        ctx: &ActorContext,
    ) {
        if let Err(e) = ctx.send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
            session_id: payload.session_id.clone(),
            text: payload.text.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit EnqueueUserMessage");
        }
    }

    /// SessionLoadCompleted: restore session state and emit follow-up commands.
    pub(in crate::feat::session::session_actor) fn handle_session_load_completed(
        &self,
        payload: &SessionLoadCompleted,
        ctx: &ActorContext,
    ) {
        let session_id = payload.session.session_id().clone();
        let strategy_id = payload.session.active_strategy().clone();

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
            state.session.sessions.insert(session_id.clone(), loaded);

            // Add a system message about the restore.
            let session = state
                .session
                .sessions
                .get_mut(&session_id)
                .expect("just inserted");
            session.push_entry(ChatEntry::system(format!("Session restored: {title_text}")));
            session.set_model(model);

            state.session.active_session = session_id.clone();
            state.session.session_loading = false;
            state.session.session_load_started_at = None;

            // Notify other actors that the active session changed.
            let _ = ctx.send_event(Event::ActiveSessionChanged(
                crate::protocol::system::ActiveSessionChanged {
                    session_id: session_id.clone(),
                },
            ));
        }

        // Serialize the strategy state for RestoreStrategyState.
        let blob = {
            let state = self.state.read();
            let session = state
                .session
                .sessions
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
    }
}
