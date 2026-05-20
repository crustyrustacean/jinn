//! Message enqueuing handlers — manage user message input, queuing, and dispatch.
//!
//! Handles the flow from user input through to prompt assembly: enqueuing messages
//! (with queueing when session is busy), updating the input buffer, pushing arbitrary
//! chat entries, and the legacy `SendMessage` compatibility shim.

use crate::common::actor::ActorContext;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::context::protocol::command::AssemblePrompt;
use crate::feat::provider::protocol::command::SendMessage;
use crate::protocol::{ChatEntry, ChatEntryKind, Command, Event};

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
}
