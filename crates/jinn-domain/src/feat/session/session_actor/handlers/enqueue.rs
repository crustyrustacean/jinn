//! Message enqueuing handlers - manage user message input, queuing, and dispatch.
//!
//! Handles the flow from user input through to prompt assembly: enqueuing messages
//! (with queueing when session is busy), updating the input buffer, pushing arbitrary
//! chat entries, and the legacy `SendMessage` compatibility shim.

use crate::common::actor::ActorContext;
use crate::feat::chat_input::protocol::command::{
    EnqueueResumeTurn, EnqueueUserMessage, PushChatEntry, SetChatInputText, SubmitSteeringMessage,
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
        tracing::info!(session = %payload.session_id, "DIAG handle_enqueue_user_message: ENTERED");
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
                // Drain any pending steering fragments into history before assembly.
                {
                    let mut state = self.state.write();
                    let session = state.session_mut_or_create(&payload.session_id);
                    if let Some(entry) = session.steering_buffer_mut().drain_into_entry() {
                        let entry_id = entry.id.clone();
                        let index = session.push_entry(entry);
                        tracing::debug!(
                            session_id = %payload.session_id,
                            entry_id = %entry_id,
                            history_index = index,
                            "drained steering entry into history at enqueue (Idle dispatch)"
                        );
                    }
                }
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

    /// EnqueueResumeTurn: re-send current history without adding a new user entry.
    ///
    /// - If the session is `Idle`, push a UI-only `System` "↻ session resumed"
    ///   marker, transition Idle → Streaming, assemble the prompt, emit
    ///   `SendToLlmProvider`, and persist. Adds no `User` entry. This mirrors
    ///   `handle_enqueue_user_message`'s inline-dispatch pattern for the Idle
    ///   branch.
    /// - If the session is busy (`Sending`/`Streaming`), silently ignored. We do
    ///   not queue resumes — the existing stream is the source of truth.
    ///
    /// The System marker is excluded from LLM context by default
    /// (see `ChatEntryKind::is_included_by_default`), so only the UI sees it.
    pub(in crate::feat::session::session_actor) async fn handle_enqueue_resume_turn(
        &self,
        payload: &EnqueueResumeTurn,
        ctx: &ActorContext,
    ) {
        use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
        use crate::feat::session::token_stats::TokenRecord;
        use crate::protocol::ChatEntry;

        // Only dispatch from Idle. Busy sessions ignore resume (no queuing).
        let should_dispatch = {
            let state = self.state.read();
            let session = state.session(&payload.session_id);
            matches!(session.phase(), PhaseKind::Idle)
        };
        if !should_dispatch {
            return;
        }

        // Push UI-only resume marker and transition Idle → Sending.
        let marker = ChatEntry::system("\u{21bb} session resumed");
        let (old_phase, new_phase) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.push_entry(marker.clone());
            let old_phase = session.phase();
            session.begin_sending();
            (old_phase, session.phase())
        };

        if old_phase != new_phase
            && let Err(e) = ctx.send_event(Event::SessionPhaseChanged(SessionPhaseChanged {
                session_id: payload.session_id.clone(),
                old_phase,
                new_phase,
            }))
        {
            tracing::warn!(err = ?e, "session-actor failed to emit SessionPhaseChanged for resume");
        }

        super::super::helpers::emit_history_appended(ctx, &payload.session_id);

        if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
            session_id: payload.session_id.clone(),
            entry: marker,
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted for resume marker");
        }

        // Drain any pending steering fragments into history before assembly.
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            if let Some(entry) = session.steering_buffer_mut().drain_into_entry() {
                let entry_id = entry.id.clone();
                let index = session.push_entry(entry);
                tracing::debug!(
                    session_id = %payload.session_id,
                    entry_id = %entry_id,
                    history_index = index,
                    "drained steering entry into history at enqueue (resume turn)"
                );
            }
        }
        // Assemble prompt and dispatch. Marker is excluded by default.
        let assembled = {
            let guard = self.state.read();
            assemble_prompt(&guard, &payload.session_id, &self.counter, None)
        };

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

        // Sending → Streaming + record outgoing token count.
        let (old_phase, new_phase) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            let old_phase = session.phase();
            session.begin_streaming();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: estimated_tokens,
                tokens_received: 0,
                cost: None,
            });
            (old_phase, session.phase())
        };

        super::super::helpers::emit_phase_changed(ctx, &payload.session_id, old_phase, new_phase);

        if let Err(e) = ctx.send_command(Command::SendToLlmProvider(SendToLlmProvider {
            session_id: payload.session_id.clone(),
            messages: assembled.messages,
            provider_id,
            estimated_tokens,
            tool_definitions: assembled.tool_definitions,
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SendToLlmProvider for resume");
        }

        self.save_active_session(&payload.session_id).await;
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

    /// SubmitSteeringMessage: append a fragment to the session's steering buffer.
    ///
    /// The buffer is drained into a `User` entry at the next prompt-assembly
    /// boundary. This handler performs no phase check - routing (queue vs steer)
    /// is the responsibility of the chat-input layer.
    pub(in crate::feat::session::session_actor) fn handle_submit_steering_message(
        &self,
        payload: &SubmitSteeringMessage,
    ) {
        let fragment_len = payload.text.len();
        let new_depth = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session
                .steering_buffer_mut()
                .push_fragment(payload.text.clone());
            session.steering_buffer().len()
        };
        tracing::debug!(
            session_id = %payload.session_id,
            fragment_len,
            new_depth,
            "steering fragment buffered"
        );
    }

    /// PushChatEntry: push entry to session history, emit ChatEntrySubmitted event,
    /// and persist the session to disk.
    pub(in crate::feat::session::session_actor) async fn handle_push_chat_entry(
        &self,
        payload: &PushChatEntry,
        ctx: &ActorContext,
    ) {
        tracing::info!(session = %payload.session_id, "DIAG handle_push_chat_entry: ENTERED");
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
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::uninlined_format_args,
        reason = "test code"
    )]
    use super::super::super::helpers::{test_actor, test_context};
    use crate::feat::chat_input::protocol::command::{
        EnqueueResumeTurn, EnqueueUserMessage, PushChatEntry, SetChatInputText,
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

    // --- Plugin dispatch tests ---

    #[tokio::test]
    async fn before_turn_no_attachments_dispatches_normally() {
        // Given an idle session with no attached plugins.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
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

        // Then the message dispatches normally.
        // Attached plugins orchestrate themselves; every enqueue becomes a direct dispatch.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(session.phase(), PhaseKind::Streaming);
    }

    // --- handle_enqueue_resume_turn ---

    #[tokio::test]
    async fn handle_enqueue_resume_turn_noop_when_streaming() {
        // Given a session already in Streaming phase.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When resume is requested.
        actor
            .handle_enqueue_resume_turn(
                &EnqueueResumeTurn {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then no commands were emitted (silent no-op).
        assert!(
            sink.commands().is_empty(),
            "expected no commands when resuming a streaming session"
        );
        // And no System marker was pushed to history.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert!(
            session.history().is_empty(),
            "history should remain empty when resume is ignored"
        );
    }

    #[tokio::test]
    async fn handle_enqueue_resume_turn_idle_dispatches_directly() {
        // Given an idle session.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let _ = state.active_session_mut();
            state.session.active_session_id().clone()
        };

        // When resume is requested.
        actor
            .handle_enqueue_resume_turn(
                &EnqueueResumeTurn {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then SendToLlmProvider is emitted directly (inline dispatch on Idle).
        let commands = sink.commands();
        let send = commands
            .iter()
            .find(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            send.is_some(),
            "expected SendToLlmProvider to be emitted directly for resume from Idle"
        );

        // And the session is now in Streaming phase.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert_eq!(
            session.phase(),
            PhaseKind::Streaming,
            "phase should be Streaming after inline resume dispatch"
        );

        // And no item was queued (we dispatched inline, not via the queue).
        assert!(
            session.queue().is_empty(),
            "resume from Idle should not enqueue; it dispatches inline"
        );

        // And exactly one System marker was pushed to history.
        let markers: Vec<_> = session
            .history()
            .iter()
            .filter(|e| matches!(e.kind, crate::protocol::ChatEntryKind::System { .. }))
            .collect();
        assert_eq!(markers.len(), 1, "expected one System marker pushed");
    }

    #[tokio::test]
    async fn handle_enqueue_resume_turn_drains_steering_buffer_before_assembly() {
        // Given an idle session with a non-empty steering buffer.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let _ = state.active_session_mut();
            let id = state.session.active_session_id().clone();
            let session = state.session_mut_or_create(&id);
            session
                .ui
                .steering_buffer
                .push_fragment("stay at the foo part".to_owned());
            id
        };

        // Sanity: buffer is non-empty before dispatch.
        {
            let state = actor.state.read();
            let session = state.session.get(&session_id).expect("session");
            assert_eq!(session.ui.steering_buffer.len(), 1);
        }

        // When resume is requested.
        actor
            .handle_enqueue_resume_turn(
                &EnqueueResumeTurn {
                    session_id: session_id.clone(),
                },
                &ctx,
            )
            .await;

        // Then the steering buffer is drained.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert!(
            session.ui.steering_buffer.is_empty(),
            "steering buffer must be drained before assembly"
        );

        // And the drained entry is now in history.
        let user_entries: Vec<_> = session
            .history()
            .iter()
            .filter(|e| matches!(e.kind, crate::protocol::ChatEntryKind::User { .. }))
            .collect();
        assert!(
            user_entries
                .iter()
                .any(|e| matches!(&e.kind, crate::protocol::ChatEntryKind::User { expanded, .. } if expanded == "stay at the foo part")),
            "drained steering entry must appear in history; history = {:?}",
            user_entries
        );

        // And SendToLlmProvider was emitted (proving dispatch ran through to assembly).
        let commands = sink.commands();
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, Command::SendToLlmProvider(_))),
            "SendToLlmProvider must be emitted after drain"
        );
    }

    #[tokio::test]
    async fn handle_enqueue_user_message_drains_steering_buffer_on_idle_dispatch() {
        // Given an idle session with a non-empty steering buffer.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session
                .steering_buffer_mut()
                .push_fragment("steer here".to_owned());
            state.session.active_session_id().clone()
        };

        // When enqueuing a user message from Idle (inline dispatch path).
        actor
            .handle_enqueue_user_message(
                &EnqueueUserMessage {
                    session_id: session_id.clone(),
                    entry: ChatEntry::user("user prompt"),
                },
                &ctx,
            )
            .await;

        // Then the steering buffer is drained before assembly.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert!(
            session.steering_buffer().is_empty(),
            "steering buffer must be drained during Idle dispatch"
        );

        // And the drained steering entry appears in history.
        let has_steering_entry = session.history().iter().any(
            |e| matches!(&e.kind, ChatEntryKind::User { expanded, .. } if expanded == "steer here"),
        );
        assert!(
            has_steering_entry,
            "drained steering entry must appear in history after Idle dispatch"
        );

        // And SendToLlmProvider was emitted (the drain happened before assembly).
        let has_send = sink
            .commands()
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            has_send,
            "SendToLlmProvider must be emitted after drain on Idle dispatch"
        );
    }
    // --- Helpers ---
}
