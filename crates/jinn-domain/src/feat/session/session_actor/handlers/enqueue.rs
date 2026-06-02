//! Message enqueuing handlers - manage user message input, queuing, and dispatch.
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
use crate::feat::session::phase_machine::PhaseKind;

/// Decision returned after inspecting session state in `EnqueueUserMessage`.
enum EnqueueAction {
    /// Session is idle - dispatch directly via assemble_prompt().
    DispatchDirectly,
    /// Session is busy - message was queued.
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
        let (action, workflow_overrides) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            let workflow_overrides: Option<crate::feat::context::assemble::AssemblyOverrides> =
                if session.is_workflow() {
                    session.core.workflow_overrides.clone()
                } else {
                    None
                };
            match session.phase() {
                PhaseKind::Idle => {
                    // --- Phase 5: Drain pending one-shots into TurnEndOneShot attachments ---
                    let drained: Vec<_> = session.ui.pending_one_shots.drain().collect();
                    for (_kind, config) in drained {
                        let aw = crate::feat::workflow::attached_workflow::AttachedWorkflow::new(
                            config,
                            crate::feat::workflow::attached_workflow::WorkflowTrigger::TurnEndOneShot,
                        );
                        session.core.attached_workflows.push(aw);
                    }

                    // --- Phase 4: BeforeTurn interception ---
                    let has_before_turn = session.core.attached_workflows.iter().any(|aw| {
                        aw.enabled
                            && matches!(aw.state, crate::feat::workflow::attached_workflow::AttachedWorkflowState::Ready)
                            && matches!(aw.trigger, crate::feat::workflow::attached_workflow::WorkflowTrigger::BeforeTurn(_))
                    });

                    if has_before_turn {
                        // Defer: store raw text, don't push to history, don't dispatch.
                        let text = match &payload.entry.kind {
                            ChatEntryKind::User { display, .. } => display.clone(),
                            _ => String::new(),
                        };
                        session.core.ephemeral.pending_user_text = Some(text);
                        // Emit FireBeforeTurn so the controller fires BeforeTurn workflows.
                        let _ = ctx.send_command(Command::FireBeforeTurn(
                            crate::feat::workflow::protocol::command::FireBeforeTurn {
                                session_id: payload.session_id.clone(),
                            },
                        ));
                        return;
                    }

                    // Normal path: set title, push entry, begin_sending.
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
                    (EnqueueAction::DispatchDirectly, workflow_overrides)
                }
                PhaseKind::Sending | PhaseKind::Streaming => {
                    session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                        payload.entry.clone(),
                    ));
                    (EnqueueAction::Queued, None)
                }
            }
        };

        match action {
            EnqueueAction::DispatchDirectly => {
                super::super::helpers::emit_history_appended(ctx, &payload.session_id);
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
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.push_entry(payload.entry.clone());
        };

        if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
            session_id: payload.session_id.clone(),
            entry: payload.entry.clone(),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
        }

        super::super::helpers::emit_history_appended(ctx, &payload.session_id);

        self.save_active_session(&payload.session_id).await;
    }

    /// SendMessage: backward compat - emit EnqueueUserMessage.
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
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::super::super::helpers::{test_actor, test_context};
    use crate::feat::chat_input::protocol::command::{
        EnqueueUserMessage, PushChatEntry, SetChatInputText,
    };
    use crate::feat::provider::protocol::command::SendMessage;
    use crate::feat::session::phase_machine::PhaseKind;
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
        assert_eq!(session.phase(), PhaseKind::Streaming);
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

        // Then the message is queued (not dispatched - phase stays Streaming).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.phase(), PhaseKind::Streaming);
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
        let send_cmd = sink.commands().iter().find_map(|c| match c {
            Command::SendToLlmProvider(s) => Some(s.provider_id.clone()),
            _ => None,
        });
        // With default model (NO_PROVIDER_ID), provider_id should be None.
        // Mutant (== -> !=) would send Some(NO_PROVIDER_ID) instead.
        assert_eq!(
            send_cmd,
            Some(None),
            "expected None provider_id for NO_PROVIDER_ID model"
        );
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
        let has_submitted = sink
            .events()
            .iter()
            .any(|e| matches!(e, Event::ChatEntrySubmitted(e) if e.session_id == session_id));
        assert!(has_submitted, "expected ChatEntrySubmitted event");

        // And HistoryAppended was emitted.
        let has_history = sink
            .events()
            .iter()
            .any(|e| matches!(e, Event::HistoryAppended(e) if e.session_id == session_id));
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
        let has_enqueue = commands
            .iter()
            .any(|c| matches!(c, Command::EnqueueUserMessage(e) if e.session_id == session_id));
        assert!(
            has_enqueue,
            "expected EnqueueUserMessage command from SendMessage"
        );
    }

    // --- Workflow BeforeTurn / One-Shot tests ---

    #[tokio::test]
    async fn before_turn_no_attachments_dispatches_normally() {
        // Given an idle session with no BeforeTurn attachments.
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
                    entry: ChatEntry::user("hello"),
                },
                &ctx,
            )
            .await;

        // Then the message dispatches normally (no FireBeforeTurn).
        let has_fire = sink
            .commands()
            .iter()
            .any(|c| matches!(c, Command::FireBeforeTurn(_)));
        assert!(!has_fire, "expected no FireBeforeTurn command");

        // And the session is streaming.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.phase(), PhaseKind::Streaming);
    }

    #[tokio::test]
    async fn before_turn_holds_text_in_pending() {
        use crate::feat::workflow::attached_workflow::{
            AttachedWorkflow, BeforeTurnMode, PromptMergeStrategy, WorkflowConfig, WorkflowTrigger,
        };

        // Given an idle session with a BeforeTurn attachment.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            let aw = AttachedWorkflow::new(
                WorkflowConfig::Consensus {
                    n: 1,
                    result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
                },
                WorkflowTrigger::BeforeTurn(BeforeTurnMode::AutoSend {
                    strategy: PromptMergeStrategy::Replace,
                }),
            );
            session.core.attached_workflows.push(aw);
            state.session.active_session_id().clone()
        };

        // When enqueuing a user message.
        actor
            .handle_enqueue_user_message(
                &EnqueueUserMessage {
                    session_id: session_id.clone(),
                    entry: ChatEntry::user("my prompt"),
                },
                &ctx,
            )
            .await;

        // Then the text is held in pending_user_text.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(
            session.core.ephemeral.pending_user_text.as_deref(),
            Some("my prompt")
        );

        // And FireBeforeTurn was emitted.
        let has_fire = sink
            .commands()
            .iter()
            .any(|c| matches!(c, Command::FireBeforeTurn(_)));
        assert!(has_fire, "expected FireBeforeTurn command");

        // And the session is NOT streaming (still idle).
        assert_eq!(session.phase(), PhaseKind::Idle);
    }

    #[tokio::test]
    async fn drain_pending_creates_one_shot_attachments() {
        use crate::feat::workflow::attached_workflow::{OneShotKind, WorkflowConfig};
        use std::collections::HashMap;

        // Given an idle session with pending one-shots.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.ui.pending_one_shots.insert(
                OneShotKind::Consensus,
                WorkflowConfig::Consensus {
                    n: 3,
                    result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
                },
            );
            state.session.active_session_id().clone()
        };

        // When enqueuing a user message.
        actor
            .handle_enqueue_user_message(
                &EnqueueUserMessage {
                    session_id: session_id.clone(),
                    entry: ChatEntry::user("test"),
                },
                &ctx,
            )
            .await;

        // Then a TurnEndOneShot attachment was created.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let one_shots: Vec<_> = session
            .core
            .attached_workflows
            .iter()
            .filter(|aw| {
                matches!(
                    aw.trigger,
                    crate::feat::workflow::attached_workflow::WorkflowTrigger::TurnEndOneShot
                )
            })
            .collect();
        assert_eq!(one_shots.len(), 1);

        // And the pending_one_shots map is cleared.
        assert!(session.ui.pending_one_shots.is_empty());
    }

    #[tokio::test]
    async fn multiple_one_shots_create_multiple_attachments() {
        use crate::feat::workflow::attached_workflow::{OneShotKind, WorkflowConfig};
        use std::collections::HashMap;

        // Given an idle session with two pending one-shots.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.ui.pending_one_shots.insert(
                OneShotKind::Consensus,
                WorkflowConfig::Consensus {
                    n: 3,
                    result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
                },
            );
            session.ui.pending_one_shots.insert(
                OneShotKind::Judge,
                WorkflowConfig::Judge {
                    prompt: String::new(),
                    approval_tool: "approve".into(),
                    result_kind: crate::feat::workflow::attached_workflow::ResultKind::System,
                },
            );
            state.session.active_session_id().clone()
        };

        // When enqueuing a user message.
        actor
            .handle_enqueue_user_message(
                &EnqueueUserMessage {
                    session_id: session_id.clone(),
                    entry: ChatEntry::user("test"),
                },
                &ctx,
            )
            .await;

        // Then two TurnEndOneShot attachments were created.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let one_shots: Vec<_> = session
            .core
            .attached_workflows
            .iter()
            .filter(|aw| {
                matches!(
                    aw.trigger,
                    crate::feat::workflow::attached_workflow::WorkflowTrigger::TurnEndOneShot
                )
            })
            .collect();
        assert_eq!(one_shots.len(), 2);
    }
    // --- Helpers ---
}
