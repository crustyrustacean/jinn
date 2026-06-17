//! Message enqueuing handlers - manage user message input, queuing, and dispatch.
//!
//! Handles the flow from user input through to prompt assembly: enqueuing messages
//! (with queueing when session is busy), updating the input buffer, pushing arbitrary
//! chat entries, and the legacy `SendMessage` compatibility shim.

use crate::common::actor_deps::BusPublish;
use crate::feat::chat_input::protocol::command::{
    EnqueueResumeTurn, EnqueueUserMessage, PushChatEntry, SetChatInputEnabled, SetChatInputText,
    SubmitSteeringMessage,
};
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::context::assemble::assemble_prompt;
use crate::feat::provider::protocol::command::{SendMessage, SendToLlmProvider};
use crate::feat::session::token_stats::TokenRecord;
use crate::protocol::{ChatEntry, ChatEntryKind};

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
    ) {
        let (action, assembly_overrides) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            let assembly_overrides: Option<crate::feat::context::assemble::AssemblyOverrides> =
                if session.is_automated() {
                    session.core.assembly_overrides.clone()
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
                    (EnqueueAction::DispatchDirectly, assembly_overrides)
                }
                PhaseKind::Sending | PhaseKind::Streaming => {
                    session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                        Box::new(payload.entry.clone()),
                    ));
                    (EnqueueAction::Queued, None)
                }
            }
        };

        match action {
            EnqueueAction::DispatchDirectly => {
                super::super::helpers::emit_history_appended(self.bus(), &payload.session_id).await;
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
                        assembly_overrides.as_ref(),
                    )
                };

                let (old_phase, new_phase) = {
                    let mut state = self.state.write();
                    let session = state.session_mut_or_create(&payload.session_id);
                    let old_phase = session.phase();
                    session.begin_streaming();
                    session.push_token_record(TokenRecord {
                        model_used: None,
                        timestamp: jiff::Timestamp::now(),
                        tokens_sent: assembled.estimated_tokens(),
                        tokens_received: 0,
                        cost: None,
                    });

                    (old_phase, session.phase())
                };
                super::super::helpers::emit_phase_changed(
                    self.bus(),
                    &payload.session_id,
                    old_phase,
                    new_phase,
                )
                .await;

                let (provider_id, model_used, reasoning_effort) = {
                    let mut state = self.state.write();
                    let session = state.session_mut(&payload.session_id);
                    let profile = session.profile_mut();
                    let global_default = self
                        .services
                        .user_preferences_storage
                        .read()
                        .reasoning
                        .default_effort;
                    let reasoning_effort =
                        crate::resolve_effort(profile.reasoning_effort, global_default);
                    if profile.model.is_no_provider() {
                        (None, None, reasoning_effort)
                    } else {
                        let resolved = profile.model.resolve_model();
                        session.set_last_token_model(resolved.clone());
                        (Some(resolved.clone()), Some(resolved), reasoning_effort)
                    }
                };

                let estimated_tokens = assembled.estimated_tokens();

                self.publish(SendToLlmProvider {
                    model_used,
                    reasoning_effort,
                    session_id: payload.session_id.clone(),
                    messages: assembled.messages,
                    provider_id,
                    estimated_tokens,
                    tool_definitions: assembled.tool_definitions,
                    dispatched_at: jiff::Timestamp::now(),
                })
                .await;

                self.publish(ChatEntrySubmitted {
                    session_id: payload.session_id.clone(),
                    entry: payload.entry.clone(),
                })
                .await;

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

        if old_phase != new_phase {
            self.publish(SessionPhaseChanged {
                session_id: payload.session_id.clone(),
                old_phase,
                new_phase,
            })
            .await;
        }

        super::super::helpers::emit_history_appended(self.bus(), &payload.session_id).await;

        self.publish(ChatEntrySubmitted {
            session_id: payload.session_id.clone(),
            entry: marker,
        })
        .await;

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

        // Resolve model under write lock (round-robin mutates index).
        // Sending → Streaming + record outgoing token count.
        let (provider_id, model_used, reasoning_effort, old_phase, new_phase) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            let reasoning_effort = {
                let profile = session.profile();
                let global_default = self
                    .services
                    .user_preferences_storage
                    .read()
                    .reasoning
                    .default_effort;
                crate::resolve_effort(profile.reasoning_effort, global_default)
            };
            let model = &mut session.profile_mut().model;
            let (provider_id, model_used) = if model.is_no_provider() {
                (None, None)
            } else {
                let resolved = model.resolve_model();
                (Some(resolved.clone()), Some(resolved))
            };
            let old_phase = session.phase();
            session.begin_streaming();
            session.push_token_record(TokenRecord {
                model_used: model_used.clone(),
                timestamp: jiff::Timestamp::now(),
                tokens_sent: assembled.estimated_tokens(),
                tokens_received: 0,
                cost: None,
            });
            (
                provider_id,
                model_used,
                reasoning_effort,
                old_phase,
                session.phase(),
            )
        };

        let estimated_tokens = assembled.estimated_tokens();

        super::super::helpers::emit_phase_changed(
            self.bus(),
            &payload.session_id,
            old_phase,
            new_phase,
        )
        .await;

        self.publish(SendToLlmProvider {
            model_used,
            reasoning_effort,
            session_id: payload.session_id.clone(),
            messages: assembled.messages,
            provider_id,
            estimated_tokens,
            tool_definitions: assembled.tool_definitions,
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;

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

    /// SetChatInputEnabled: enable or disable editing for the session's input box.
    pub(in crate::feat::session::session_actor) fn handle_set_chat_input_enabled(
        &self,
        payload: &SetChatInputEnabled,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.chat_input_mut().set_enabled(payload.enabled);
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
    ) {
        tracing::debug!(
            session_id = %payload.session_id,
            kind = %payload.entry.kind_str(),
            preview = %payload.entry.text().chars().take(60).collect::<String>(),
            "handle_push_chat_entry"
        );
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.push_entry(payload.entry.clone());
        };

        self.publish(ChatEntrySubmitted {
            session_id: payload.session_id.clone(),
            entry: payload.entry.clone(),
        })
        .await;

        super::super::helpers::emit_history_appended(self.bus(), &payload.session_id).await;

        self.save_active_session(&payload.session_id).await;
    }

    /// SendMessage: backward compat - emit EnqueueUserMessage.
    pub(in crate::feat::session::session_actor) async fn handle_send_message(
        &self,
        payload: &SendMessage,
    ) {
        self.publish(EnqueueUserMessage {
            session_id: payload.session_id.clone(),
            entry: ChatEntry::user(&payload.text),
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::uninlined_format_args,
        reason = "test code"
    )]

    use crate::common::services::BusAudit;
    use crate::feat::chat_input::protocol::command::{
        EnqueueResumeTurn, EnqueueUserMessage, PushChatEntry, SetChatInputText,
    };
    use crate::feat::provider::protocol::command::SendMessage;
    use crate::feat::session::phase_machine::PhaseKind;
    use crate::protocol::{ChatEntry, ChatEntryKind};

    async fn create_actor() -> (
        super::super::super::SessionPersistenceActor,
        crate::common::state::State,
        BusAudit,
    ) {
        let state = crate::common::state::State::new(crate::common::app_state::AppState::default());
        let (actor, audit) = super::super::super::helpers::test_actor_recording().await;
        let actor = super::super::super::SessionPersistenceActor {
            state: state.clone(),
            ..actor
        };
        (actor, state, audit)
    }

    // --- handle_enqueue_user_message ---

    #[tokio::test]
    async fn handle_enqueue_user_message_dispatches_when_idle() {
        // Given an idle session.
        let (actor, state, audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let _session = guard.active_session_mut();
            guard.session.active_session_id().clone()
        };

        // When enqueuing a user message.
        actor
            .handle_enqueue_user_message(&EnqueueUserMessage {
                session_id: session_id.clone(),
                entry: ChatEntry::user("hello world"),
            })
            .await;

        // Then the message is dispatched (history has the entry, phase is streaming).
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert_eq!(session.phase(), PhaseKind::Streaming);
        assert_eq!(session.history().len(), 1);
        assert!(
            matches!(&session.history()[0].kind, ChatEntryKind::User { display, .. } if display == "hello world"),
            "expected user entry in history"
        );

        // And SendToLlmProvider was emitted.
        assert!(
            audit.contains_name("SendToLlmProvider"),
            "expected SendToLlmProvider command"
        );
    }

    #[tokio::test]
    async fn handle_enqueue_user_message_sets_title_from_first_message() {
        // Given a new session with no title.
        let (actor, state, _audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let _ = guard.active_session_mut();
            guard.session.active_session_id().clone()
        };

        // When enqueuing the first user message.
        actor
            .handle_enqueue_user_message(&EnqueueUserMessage {
                session_id: session_id.clone(),
                entry: ChatEntry::user("My First Question"),
            })
            .await;

        // Then the title is set from the first line of the message.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert_eq!(session.title(), Some("My First Question"));
    }

    #[tokio::test]
    async fn handle_enqueue_user_message_queues_when_busy() {
        // Given a session in Streaming phase (busy).
        let (actor, state, _audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let session = guard.active_session_mut();
            session.begin_streaming();
            guard.session.active_session_id().clone()
        };

        // When enqueuing a user message.
        actor
            .handle_enqueue_user_message(&EnqueueUserMessage {
                session_id: session_id.clone(),
                entry: ChatEntry::user("queued msg"),
            })
            .await;

        // Then the message is queued (not dispatched - phase stays Streaming).
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert_eq!(session.phase(), PhaseKind::Streaming);
        // No history entry because the message was queued, not pushed.
        assert_eq!(session.history().len(), 0);
        // The queue should have the message.
        assert_eq!(session.queue().len(), 1);
    }

    #[tokio::test]
    async fn handle_enqueue_user_message_no_provider_sends_none_provider_id() {
        // Given a session with default model (NO_PROVIDER_ID).
        let (actor, state, audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let _ = guard.active_session_mut();
            guard.session.active_session_id().clone()
        };

        // When enqueuing a message.
        actor
            .handle_enqueue_user_message(&EnqueueUserMessage {
                session_id: session_id.clone(),
                entry: ChatEntry::user("test"),
            })
            .await;

        // Then SendToLlmProvider was emitted (provider_id not checked here, just presence).
        assert!(
            audit.contains_name("SendToLlmProvider"),
            "expected SendToLlmProvider command"
        );
    }

    // --- handle_set_chat_input_text ---

    #[tokio::test]
    async fn handle_set_chat_input_text_updates_buffer() {
        // Given a session.
        let (actor, state, _audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let _ = guard.active_session_mut();
            guard.session.active_session_id().clone()
        };

        // When setting the input text.
        actor.handle_set_chat_input_text(&SetChatInputText {
            session_id: session_id.clone(),
            text: "new input text".to_owned(),
        });

        // Then the input buffer is updated.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert_eq!(session.chat_input().text(), "new input text");
    }

    // --- handle_push_chat_entry ---

    #[tokio::test]
    async fn handle_push_chat_entry_pushes_and_emits() {
        // Given a session.
        let (actor, state, audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let _ = guard.active_session_mut();
            guard.session.active_session_id().clone()
        };

        // When pushing a chat entry.
        let entry = ChatEntry::user("pushed");
        actor
            .handle_push_chat_entry(&PushChatEntry {
                session_id: session_id.clone(),
                entry: entry.clone(),
            })
            .await;

        // Then the entry is in history.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert_eq!(session.history().len(), 1);
        assert!(
            matches!(&session.history()[0].kind, ChatEntryKind::User { display, .. } if display == "pushed"),
            "expected pushed entry"
        );

        // And ChatEntrySubmitted event was emitted.
        assert!(
            audit.contains_name("ChatEntrySubmitted"),
            "expected ChatEntrySubmitted event"
        );

        // And HistoryAppended was emitted.
        assert!(
            audit.contains_name("HistoryAppended"),
            "expected HistoryAppended event"
        );
    }

    // --- handle_send_message ---

    #[tokio::test]
    async fn handle_send_message_emits_enqueue_user_message() {
        // Given a test context.
        let (actor, _state, audit) = create_actor().await;
        let session_id = crate::protocol::SessionId::new();

        // When calling handle_send_message.
        actor
            .handle_send_message(&SendMessage {
                session_id: session_id.clone(),
                text: "legacy message".to_owned(),
            })
            .await;

        // Then EnqueueUserMessage command was emitted.
        assert!(
            audit.contains_name("EnqueueUserMessage"),
            "expected EnqueueUserMessage command from SendMessage"
        );
    }

    // --- Plugin dispatch tests ---

    #[tokio::test]
    async fn before_turn_no_attachments_dispatches_normally() {
        // Given an idle session with no attached plugins.
        let (actor, state, _audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let _session = guard.active_session_mut();
            guard.session.active_session_id().clone()
        };

        // When enqueuing a user message.
        actor
            .handle_enqueue_user_message(&EnqueueUserMessage {
                session_id: session_id.clone(),
                entry: ChatEntry::user("hello"),
            })
            .await;

        // Then the message dispatches normally.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert_eq!(session.phase(), PhaseKind::Streaming);
    }

    // --- handle_enqueue_resume_turn ---

    #[tokio::test]
    async fn handle_enqueue_resume_turn_noop_when_streaming() {
        // Given a session already in Streaming phase.
        let (actor, state, audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let session = guard.active_session_mut();
            session.begin_streaming();
            guard.session.active_session_id().clone()
        };

        // When resume is requested.
        actor
            .handle_enqueue_resume_turn(&EnqueueResumeTurn {
                session_id: session_id.clone(),
            })
            .await;

        // Then no commands were emitted (silent no-op).
        assert!(
            audit.is_empty(),
            "expected no commands when resuming a streaming session"
        );
        // And no System marker was pushed to history.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert!(
            session.history().is_empty(),
            "history should remain empty when resume is ignored"
        );
    }

    #[tokio::test]
    async fn handle_enqueue_resume_turn_idle_dispatches_directly() {
        // Given an idle session.
        let (actor, state, audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let _ = guard.active_session_mut();
            guard.session.active_session_id().clone()
        };

        // When resume is requested.
        actor
            .handle_enqueue_resume_turn(&EnqueueResumeTurn {
                session_id: session_id.clone(),
            })
            .await;

        // Then SendToLlmProvider is emitted directly (inline dispatch on Idle).
        assert!(
            audit.contains_name("SendToLlmProvider"),
            "expected SendToLlmProvider to be emitted directly for resume from Idle"
        );

        // And the session is now in Streaming phase.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
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
        let (actor, state, audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let _ = guard.active_session_mut();
            let id = guard.session.active_session_id().clone();
            let session = guard.session_mut_or_create(&id);
            session
                .ui
                .steering_buffer
                .push_fragment("stay at the foo part".to_owned());
            id
        };

        // Sanity: buffer is non-empty before dispatch.
        {
            let guard = state.read();
            let session = guard.session.get(&session_id).expect("session");
            assert_eq!(session.ui.steering_buffer.len(), 1);
        }

        // When resume is requested.
        actor
            .handle_enqueue_resume_turn(&EnqueueResumeTurn {
                session_id: session_id.clone(),
            })
            .await;

        // Then the steering buffer is drained.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
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
        assert!(
            audit.contains_name("SendToLlmProvider"),
            "SendToLlmProvider must be emitted after drain"
        );
    }

    #[tokio::test]
    async fn handle_enqueue_user_message_drains_steering_buffer_on_idle_dispatch() {
        // Given an idle session with a non-empty steering buffer.
        let (actor, state, audit) = create_actor().await;
        let session_id = {
            let mut guard = state.write();
            let session = guard.active_session_mut();
            session
                .steering_buffer_mut()
                .push_fragment("steer here".to_owned());
            guard.session.active_session_id().clone()
        };

        // When enqueuing a user message from Idle (inline dispatch path).
        actor
            .handle_enqueue_user_message(&EnqueueUserMessage {
                session_id: session_id.clone(),
                entry: ChatEntry::user("user prompt"),
            })
            .await;

        // Then the steering buffer is drained before assembly.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
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
        assert!(
            audit.contains_name("SendToLlmProvider"),
            "SendToLlmProvider must be emitted after drain on Idle dispatch"
        );
    }
}
