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
use crate::feat::context::assemble::assemble_prompt;
use crate::feat::provider::protocol::command::{SendMessage, SendToLlmProvider};
use crate::feat::session::token_stats::TokenRecord;
use crate::protocol::{ChatEntry, ChatEntryKind, Command, Event};

use super::super::SessionPersistenceActor;
use crate::feat::session::chat_session::SessionPhase;

/// Decision returned after inspecting session state in `EnqueueUserMessage`.
enum EnqueueAction {
    /// Session is idle — dispatch directly via assemble_prompt().
    DispatchDirectly,
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
                    let total_tokens = super::super::helpers::estimate_total_tokens(session);
                    (EnqueueAction::DispatchDirectly, total_tokens)
                }
                SessionPhase::Sending
                | SessionPhase::Streaming
                | SessionPhase::Assembling
                | SessionPhase::Compacting
                | SessionPhase::TearingDown => {
                    session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                        payload.entry.clone(),
                    ));
                    (EnqueueAction::Queued, 0)
                }
            }
        };

        let (action, total_tokens) = action;

        match action {
            EnqueueAction::DispatchDirectly => {
                super::super::helpers::emit_history_appended(
                    ctx,
                    &payload.session_id,
                    total_tokens,
                );
                // Assemble the prompt directly and emit SendToLlmProvider.
                let assembled = {
                    let guard = self.state.read();
                    assemble_prompt(&guard, &payload.session_id, &self.counter)
                };

                let (old_phase, new_phase) = {
                    let mut state = self.state.write();
                    let session = state.session_mut_or_create(&payload.session_id);
                    let old_phase = session.phase();
                    session.begin_streaming();
                    session.push_token_record(TokenRecord {
                        timestamp: jiff::Timestamp::now(),
                        tokens_sent: assembled.estimated_tokens(),
                        tokens_received: 0,
                        cost: None,
                    });
                    session.set_context_size(assembled.estimated_tokens());
                    (old_phase, session.phase())
                };
                super::super::helpers::emit_phase_changed(
                    ctx,
                    &payload.session_id,
                    old_phase,
                    new_phase,
                );

                let provider_id = {
                    let state = self.state.read();
                    let model = state.session(&payload.session_id).profile().model.clone();
                    if model == crate::feat::provider_infra::NO_PROVIDER_ID {
                        None
                    } else {
                        Some(model)
                    }
                };

                let estimated_tokens = assembled.estimated_tokens();

                if let Err(e) = ctx.send_command(Command::SendToLlmProvider(SendToLlmProvider {
                    session_id: payload.session_id.clone(),
                    messages: assembled.messages,
                    provider_id,
                    estimated_tokens,
                    tool_definitions: assembled.tool_definitions,
                })) {
                    tracing::warn!(
                        err = ?e,
                        "session-actor failed to emit SendToLlmProvider"
                    );
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
        let total_tokens = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.push_entry(payload.entry.clone());
            super::super::helpers::estimate_total_tokens(session)
        };

        if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
            session_id: payload.session_id.clone(),
            entry: payload.entry.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
        }

        super::super::helpers::emit_history_appended(ctx, &payload.session_id, total_tokens);

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
