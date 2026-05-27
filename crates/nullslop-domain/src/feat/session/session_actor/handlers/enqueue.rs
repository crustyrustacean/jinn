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
    #[expect(clippy::too_many_lines, reason = "1 line over limit")]
    pub(in crate::feat::session::session_actor) async fn handle_enqueue_user_message(
        &self,
        payload: &EnqueueUserMessage,
        ctx: &ActorContext,
    ) {
        let (action, total_tokens, workflow_overrides) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            let workflow_overrides: Option<crate::feat::context::assemble::AssemblyOverrides> =
                if session.is_workflow() {
                    session.core.workflow_overrides.clone()
                } else {
                    None
                };
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
                    (
                        EnqueueAction::DispatchDirectly,
                        total_tokens,
                        workflow_overrides,
                    )
                }
                SessionPhase::Sending
                | SessionPhase::Streaming
                | SessionPhase::Assembling
                | SessionPhase::Compacting
                | SessionPhase::TearingDown => {
                    session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                        payload.entry.clone(),
                    ));
                    (EnqueueAction::Queued, 0usize, None)
                }
            }
        };

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
                    assemble_prompt(
                        &guard,
                        &payload.session_id,
                        &self.counter,
                        workflow_overrides.as_ref(),
                    )
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::super::super::helpers::{test_actor, test_context};
    use crate::feat::chat_input::protocol::command::{
        EnqueueUserMessage, PushChatEntry, SetChatInputText,
    };
    use crate::feat::provider::protocol::command::SendMessage;
    use crate::feat::session::chat_session::SessionPhase;
    use crate::protocol::{ChatEntry, ChatEntryKind, Command, Event};

    // --- handle_enqueue_user_message ---

    #[tokio::test]
    async fn handle_enqueue_user_message_dispatches_when_idle() {
        // Given an idle session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let _session = state.active_session_mut();
            state.session.active_session_id().clone()
        };

        // When enqueuing a user message.
        actor
            .handle_enqueue_user_message(
                &EnqueueUserMessage {
                    session_id: session_id.clone(),
                    entry: ChatEntry::user("hello world"),
                },
                &ctx,
            )
            .await;

        // Then the message is dispatched (history has the entry, phase is streaming).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.phase(), SessionPhase::Streaming);
        assert_eq!(session.history().len(), 1);
        assert!(
            matches!(&session.history()[0].kind, ChatEntryKind::User { display, .. } if display == "hello world"),
            "expected user entry in history"
        );

        // And SendToLlmProvider was emitted.
        let has_send = sink
            .commands()
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(has_send, "expected SendToLlmProvider command");
    }

    #[tokio::test]
    async fn handle_enqueue_user_message_sets_title_from_first_message() {
        // Given a new session with no title.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let _ = state.active_session_mut();
            state.session.active_session_id().clone()
        };

        // When enqueuing the first user message.
        actor
            .handle_enqueue_user_message(
                &EnqueueUserMessage {
                    session_id: session_id.clone(),
                    entry: ChatEntry::user("My First Question"),
                },
                &ctx,
            )
            .await;

        // Then the title is set from the first line of the message.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.title(), Some("My First Question"));
    }

    #[tokio::test]
    async fn handle_enqueue_user_message_queues_when_busy() {
        // Given a session in Streaming phase (busy).
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When enqueuing a user message.
        actor
            .handle_enqueue_user_message(
                &EnqueueUserMessage {
                    session_id: session_id.clone(),
                    entry: ChatEntry::user("queued msg"),
                },
                &ctx,
            )
            .await;

        // Then the message is queued (not dispatched — phase stays Streaming).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.phase(), SessionPhase::Streaming);
        // No history entry because the message was queued, not pushed.
        assert_eq!(session.history().len(), 0);
        // The queue should have the message.
        assert_eq!(session.queue().len(), 1);
    }

    #[tokio::test]
    async fn handle_enqueue_user_message_no_provider_sends_none_provider_id() {
        // Given a session with default model (NO_PROVIDER_ID).
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let _ = state.active_session_mut();
            state.session.active_session_id().clone()
        };

        // When enqueuing a message.
        actor
            .handle_enqueue_user_message(
                &EnqueueUserMessage {
                    session_id: session_id.clone(),
                    entry: ChatEntry::user("test"),
                },
                &ctx,
            )
            .await;

        // Then SendToLlmProvider has provider_id = None (because model == NO_PROVIDER_ID).
        let send_cmd = sink
            .commands()
            .iter()
            .find_map(|c| match c {
                Command::SendToLlmProvider(s) => Some(s.provider_id.clone()),
                _ => None,
            });
        // With default model (NO_PROVIDER_ID), provider_id should be None.
        // Mutant (== -> !=) would send Some(NO_PROVIDER_ID) instead.
        assert_eq!(send_cmd, Some(None), "expected None provider_id for NO_PROVIDER_ID model");
    }

    // --- handle_set_chat_input_text ---

    #[tokio::test]
    async fn handle_set_chat_input_text_updates_buffer() {
        // Given a session.
        let actor = test_actor();
        let session_id = {
            let mut state = actor.state.write();
            let _ = state.active_session_mut();
            state.session.active_session_id().clone()
        };

        // When setting the input text.
        actor.handle_set_chat_input_text(&SetChatInputText {
            session_id: session_id.clone(),
            text: "new input text".to_owned(),
        });

        // Then the input buffer is updated.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.chat_input().text(), "new input text");
    }

    // --- handle_push_chat_entry ---

    #[tokio::test]
    async fn handle_push_chat_entry_pushes_and_emits() {
        // Given a session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let _ = state.active_session_mut();
            state.session.active_session_id().clone()
        };

        // When pushing a chat entry.
        let entry = ChatEntry::user("pushed");
        actor
            .handle_push_chat_entry(
                &PushChatEntry {
                    session_id: session_id.clone(),
                    entry: entry.clone(),
                },
                &ctx,
            )
            .await;

        // Then the entry is in history.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.history().len(), 1);
        assert!(
            matches!(&session.history()[0].kind, ChatEntryKind::User { display, .. } if display == "pushed"),
            "expected pushed entry"
        );

        // And ChatEntrySubmitted event was emitted.
        let has_submitted = sink.events().iter().any(|e| {
            matches!(e, Event::ChatEntrySubmitted(e) if e.session_id == session_id)
        });
        assert!(has_submitted, "expected ChatEntrySubmitted event");

        // And HistoryAppended was emitted.
        let has_history = sink.events().iter().any(|e| {
            matches!(e, Event::HistoryAppended(e) if e.session_id == session_id)
        });
        assert!(has_history, "expected HistoryAppended event");
    }

    // --- handle_send_message ---

    #[tokio::test]
    async fn handle_send_message_emits_enqueue_user_message() {
        // Given a test context.
        let _actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = crate::protocol::SessionId::new();

        // When calling handle_send_message.
        crate::feat::session::session_actor::SessionPersistenceActor::handle_send_message(
            &SendMessage {
                session_id: session_id.clone(),
                text: "legacy message".to_owned(),
            },
            &ctx,
        );

        // Then EnqueueUserMessage command was emitted.
        let commands = sink.commands();
        let has_enqueue = commands.iter().any(|c| {
            matches!(c, Command::EnqueueUserMessage(e) if e.session_id == session_id)
        });
        assert!(has_enqueue, "expected EnqueueUserMessage command from SendMessage");
    }

    // --- Helpers ---
}
