//! Command handlers — process session lifecycle commands.

use crate::actor::ActorContext;
use crate::protocol::chat_input::{
    ChatEntrySubmitted, EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::protocol::context::{AssemblePrompt, RestoreStrategyState, SwitchPromptStrategy};
use crate::protocol::provider::SendMessage;
use crate::protocol::session::SessionLoadCompleted;
use crate::protocol::tool::PushToolResult;
use crate::protocol::{ChatEntry, Command, Event};

use super::super::SessionPersistenceActor;

/// Decision returned after inspecting session state in `EnqueueUserMessage`.
enum EnqueueAction {
    /// Session is idle — dispatch prompt assembly.
    AssemblePrompt,
    /// Session is streaming — message was queued.
    Queued,
    /// Session is busy (sending or assembling) — put text back in the input box.
    SetInputText(String),
}

impl SessionPersistenceActor {
    /// EnqueueUserMessage: if idle → assemble prompt; if streaming → queue;
    /// otherwise → set input text.
    pub(in crate::session::actor) fn handle_enqueue_user_message(
        &self,
        payload: &EnqueueUserMessage,
        ctx: &ActorContext,
    ) {
        let action = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            if session.is_idle() {
                session.push_entry(ChatEntry::user(&payload.text));
                session.begin_sending();
                EnqueueAction::AssemblePrompt
            } else if session.is_streaming() {
                session.enqueue_message(payload.text.clone());
                EnqueueAction::Queued
            } else {
                EnqueueAction::SetInputText(payload.text.clone())
            }
        };

        let (history, model_name) = match action {
            EnqueueAction::AssemblePrompt => {
                let state = self.state.read();
                let history = state.session(&payload.session_id).history().to_vec();
                let model_name = state.provider.active_provider.clone();
                (history, model_name)
            }
            EnqueueAction::Queued | EnqueueAction::SetInputText(_) => (vec![], String::new()),
        };

        match action {
            EnqueueAction::AssemblePrompt => {
                if let Err(e) = ctx.send_command(Command::AssemblePrompt {
                    payload: AssemblePrompt {
                        session_id: payload.session_id.clone(),
                        history,
                        tools: vec![],
                        model_name,
                    },
                }) {
                    tracing::warn!(err = ?e, "session-actor failed to emit AssemblePrompt");
                }

                if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted {
                    payload: ChatEntrySubmitted {
                        session_id: payload.session_id.clone(),
                        entry: ChatEntry::user(&payload.text),
                    },
                }) {
                    tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
                }
            }
            EnqueueAction::Queued => {}
            EnqueueAction::SetInputText(text) => {
                if let Err(e) = ctx.send_command(Command::SetChatInputText {
                    payload: SetChatInputText {
                        session_id: payload.session_id.clone(),
                        text,
                    },
                }) {
                    tracing::warn!(err = ?e, "session-actor failed to emit SetChatInputText");
                }
            }
        }
    }

    /// SetChatInputText: update the session's input buffer.
    pub(in crate::session::actor) fn handle_set_chat_input_text(&self, payload: &SetChatInputText) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.chat_input_mut().replace_all(payload.text.clone());
    }

    /// PushChatEntry: push entry to session history, emit ChatEntrySubmitted event.
    pub(in crate::session::actor) fn handle_push_chat_entry(
        &self,
        payload: &PushChatEntry,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.push_entry(payload.entry.clone());
        }

        if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted {
            payload: ChatEntrySubmitted {
                session_id: payload.session_id.clone(),
                entry: payload.entry.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
        }
    }

    /// PushToolResult: add tool result to session history.
    pub(in crate::session::actor) fn handle_push_tool_result(&self, payload: &PushToolResult) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.push_entry(ChatEntry::tool_result(
            &payload.result.tool_call_id,
            &payload.result.name,
            &payload.result.content,
            payload.result.success,
        ));
    }

    /// SendMessage: backward compat — emit EnqueueUserMessage.
    pub(in crate::session::actor) fn handle_send_message(
        payload: &SendMessage,
        ctx: &ActorContext,
    ) {
        if let Err(e) = ctx.send_command(Command::EnqueueUserMessage {
            payload: EnqueueUserMessage {
                session_id: payload.session_id.clone(),
                text: payload.text.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit EnqueueUserMessage");
        }
    }

    /// SessionLoadCompleted: restore session state and emit follow-up commands.
    pub(in crate::session::actor) fn handle_session_load_completed(
        &self,
        payload: &SessionLoadCompleted,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.restore_history(payload.history.clone());
            state.session.active_session = payload.session_id.clone();
            state.session.session_loading = false;
        }

        if let Err(e) = ctx.send_command(Command::RestoreStrategyState {
            payload: RestoreStrategyState {
                session_id: payload.session_id.clone(),
                strategy_id: payload.active_strategy.clone(),
                blob: payload
                    .blobs
                    .get(&payload.active_strategy.to_string())
                    .cloned()
                    .unwrap_or(serde_json::json!({})),
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit RestoreStrategyState");
        }

        if let Err(e) = ctx.send_command(Command::SwitchPromptStrategy {
            payload: SwitchPromptStrategy {
                session_id: payload.session_id.clone(),
                strategy_id: payload.active_strategy.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit SwitchPromptStrategy");
        }
    }
}
