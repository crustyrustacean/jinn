// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Queue actor - sole owner of turn dispatch queue consumption.
//!
//! Subscribes to [`SessionPhaseChanged`] events.
//! When a session transitions to `Idle`, pops the next item from the turn queue
//! and dispatches it.
//!
//! # Dispatch behavior
//!
//! - `UserMessage` → push entry, set title, begin sending, call `assemble_prompt()` + emit `SendToLlmProvider`
//! - `ToolContinuation` → call `assemble_prompt()` + emit `SendToLlmProvider`

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::context::assemble::assemble_prompt;
use crate::feat::context::strategy::token_estimator::TiktokenCounter;
use crate::feat::provider::protocol::command::SendToLlmProvider;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::session::queue_item::QueueItem;
use crate::feat::session_lifecycle::protocol::command::PersistSession;
use crate::protocol::{Command, Event};

/// The queue actor.
///
/// The sole consumer of the turn dispatch queue. Reacts to session phase
/// transitions to `Idle` by popping and dispatching queued items.
pub struct QueueActor {
    /// Shared application state (read/write access to session queue and data).
    state: State,
    /// Token counter for recording token usage in the session ledger.
    counter: TiktokenCounter,
}

/// Dependencies for [`QueueActor`].
pub struct QueueActorDeps {
    /// Shared application state.
    pub state: State,
    /// Token counter for usage tracking.
    pub counter: TiktokenCounter,
}

impl Actor for QueueActor {
    type Message = NoDirectMsg;
    type Deps = QueueActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Dispatches queued turns when sessions become idle");
        ctx.subscribe_event::<SessionPhaseChanged>();

        Self {
            state: deps.state,
            counter: deps.counter,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::SessionPhaseChanged(payload)) => {
                self.handle_session_phase_changed(&payload, ctx).await;
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::System(_)
            | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl QueueActor {
    /// Handle `SessionPhaseChanged` - dispatch on phase transitions.
    async fn handle_session_phase_changed(
        &self,
        payload: &SessionPhaseChanged,
        ctx: &ActorContext,
    ) {
        match (payload.old_phase, payload.new_phase) {
            (_, PhaseKind::Idle) => {
                self.handle_idle_transition(&payload.session_id, ctx).await;
            }
            _ => {}
        }
    }

    /// Handle Idle transition - pop and dispatch the next queued item.
    ///
    /// If the turn queue is empty, checks the steering buffer as a fallback.
    /// When steering fragments are present, drains them into history and
    /// dispatches a resume turn so the steering messages reach the LLM.
    async fn handle_idle_transition(
        &self,
        session_id: &crate::protocol::SessionId,
        ctx: &ActorContext,
    ) {
        let item = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            session.dequeue()
        };

        if let Some(item) = item {
            match item {
                QueueItem::UserMessage(entry) => {
                    self.dispatch_user_message(session_id, &entry, ctx).await;
                }
                QueueItem::ToolContinuation => {
                    self.dispatch_tool_continuation(session_id, ctx).await;
                }
            }
            return;
        }

        // Queue was empty — check steering buffer as fallback.
        let has_steering = {
            let state = self.state.read();
            let session = state.session(session_id);
            !session.steering_buffer().is_empty()
        };

        if has_steering {
            self.dispatch_resume(session_id, ctx, "steering buffer")
                .await;
        }
    }

    /// Dispatch a user message: push to history, set title, begin sending,
    /// assemble prompt, emit SendToLlmProvider, emit ChatEntrySubmitted, emit PersistSession.
    #[expect(clippy::unused_async, reason = "trait contract requires async")]
    async fn dispatch_user_message(
        &self,
        session_id: &crate::protocol::SessionId,
        entry: &crate::protocol::ChatEntry,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            if session.title().is_none() {
                let title = match &entry.kind {
                    crate::protocol::ChatEntryKind::User { display, .. } => {
                        display.lines().next().unwrap_or("").to_owned()
                    }
                    _ => String::new(),
                };
                session.set_title(title);
            }
            session.push_entry(entry.clone());
            // Drain any pending steering fragments into history before assembly.
            if let Some(steer_entry) = session.steering_buffer_mut().drain_into_entry() {
                let entry_id = steer_entry.id.clone();
                session.push_entry(steer_entry);
                tracing::debug!(
                    session_id = %session_id,
                    entry_id = %entry_id,
                    "drained steering entry into history at queue_actor::dispatch_user_message"
                );
            }
            session.begin_sending();
        }

        let assembled = {
            let guard = self.state.read();
            assemble_prompt(&guard, session_id, &self.counter, None)
        };

        let provider_id = {
            let state = self.state.read();
            let model = state.session(session_id).profile().model.clone();
            if model == crate::feat::provider_infra::NO_PROVIDER_ID {
                None
            } else {
                Some(model)
            }
        };

        let estimated_tokens = assembled.estimated_tokens();

        if let Err(e) = ctx.send_command(Command::SendToLlmProvider(SendToLlmProvider {
            session_id: session_id.clone(),
            messages: assembled.messages,
            provider_id,
            estimated_tokens,
            tool_definitions: assembled.tool_definitions,
            dispatched_at: jiff::Timestamp::now(),
        })) {
            tracing::warn!(err = ?e, "queue-actor failed to emit SendToLlmProvider");
        }

        if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
            session_id: session_id.clone(),
            entry: entry.clone(),
        })) {
            tracing::warn!(err = ?e, "queue-actor failed to emit ChatEntrySubmitted");
        }

        if let Err(e) = ctx.send_command(Command::PersistSession(PersistSession {
            session_id: session_id.clone(),
        })) {
            tracing::warn!(err = ?e, "queue-actor failed to emit PersistSession");
        }
    }

    /// Dispatch a tool continuation: assemble prompt and emit SendToLlmProvider.
    ///
    /// See [`Self::dispatch_resume`] for the shared dispatch body.
    async fn dispatch_tool_continuation(
        &self,
        session_id: &crate::protocol::SessionId,
        ctx: &ActorContext,
    ) {
        self.dispatch_resume(session_id, ctx, "tool continuation")
            .await;
    }

    /// Shared dispatch body for tool-continuation and manual-resume paths:
    /// re-assemble prompt from current history and emit `SendToLlmProvider`.
    #[expect(clippy::unused_async, reason = "trait contract requires async")]
    async fn dispatch_resume(
        &self,
        session_id: &crate::protocol::SessionId,
        ctx: &ActorContext,
        label: &str,
    ) {
        // Drain any pending steering fragments into history before assembly.
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            if let Some(entry) = session.steering_buffer_mut().drain_into_entry() {
                let entry_id = entry.id.clone();
                session.push_entry(entry);
                tracing::debug!(
                    session_id = %session_id,
                    entry_id = %entry_id,
                    label = %label,
                    "drained steering entry into history at queue_actor::dispatch_resume"
                );
            }
        }
        let assembled = {
            let guard = self.state.read();
            assemble_prompt(&guard, session_id, &self.counter, None)
        };

        let provider_id = {
            let state = self.state.read();
            let model = state.session(session_id).profile().model.clone();
            if model == crate::feat::provider_infra::NO_PROVIDER_ID {
                None
            } else {
                Some(model)
            }
        };

        let estimated_tokens = assembled.estimated_tokens();

        if let Err(e) = ctx.send_command(Command::SendToLlmProvider(SendToLlmProvider {
            session_id: session_id.clone(),
            messages: assembled.messages,
            provider_id,
            estimated_tokens,
            tool_definitions: assembled.tool_definitions,
            dispatched_at: jiff::Timestamp::now(),
        })) {
            tracing::warn!(
                err = ?e,
                label,
                "queue-actor failed to emit SendToLlmProvider"
            );
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
    use crate::common::actor::{ActorContext, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::feat::session::phase_machine::PhaseKind;
    use crate::protocol::ChatEntry;

    fn test_actor() -> QueueActor {
        QueueActor {
            state: crate::common::state::State::new(AppState::default()),
            counter: crate::feat::context::strategy::token_estimator::TiktokenCounter::o200k_base(),
        }
    }

    fn test_context() -> (std::sync::Arc<RecordingSink>, ActorContext) {
        let sink = std::sync::Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-queue-actor", sink.clone());
        (sink, ctx)
    }

    #[tokio::test]
    async fn session_phase_changed_idle_pops_user_message_from_queue() {
        // Given a session with a queued user message in Idle phase.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user(
                "queued message",
            ))));
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Idle phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Sending,
            new_phase: PhaseKind::Idle,
        };
        actor.handle_session_phase_changed(&payload, &ctx).await;

        // Then SendToLlmProvider was emitted for the queued message.
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            has_send,
            "expected SendToLlmProvider command for queued user message"
        );

        // And the queue is empty.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            session.queue_len() == 0,
            "expected queue to be empty after dispatch"
        );
    }

    #[tokio::test]
    async fn session_phase_changed_non_idle_does_not_pop_queue() {
        // Given a session with a queued user message in non-Idle phase.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user(
                "queued message",
            ))));
            session.begin_sending();
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Sending phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Sending,
        };
        actor.handle_session_phase_changed(&payload, &ctx).await;

        // Then no SendToLlmProvider was emitted.
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            !has_send,
            "expected no SendToLlmProvider for non-Idle phase"
        );

        // And the queue still has the item.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 1);
    }

    #[tokio::test]
    async fn session_phase_changed_idle_with_empty_queue_is_noop() {
        // Given a session with an empty queue.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Idle phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        };
        actor.handle_session_phase_changed(&payload, &ctx).await;

        // Then no commands were emitted.
        let commands = sink.commands();
        assert!(commands.is_empty(), "expected no commands for empty queue");
    }

    #[tokio::test]
    async fn dispatch_user_message_emits_chat_entry_submitted() {
        // Given a session in Idle phase.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When dispatching a user message.
        actor
            .dispatch_user_message(&session_id, &ChatEntry::user("hello"), &ctx)
            .await;

        // Then ChatEntrySubmitted was emitted.
        let events = sink.events();
        let has_submitted = events
            .iter()
            .any(|e| matches!(e, Event::ChatEntrySubmitted(_)));
        assert!(has_submitted, "expected ChatEntrySubmitted event");
    }

    #[tokio::test]
    async fn dispatch_user_message_sets_title_on_first_message() {
        // Given a session with no title.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When dispatching a user message.
        actor
            .dispatch_user_message(&session_id, &ChatEntry::user("my new chat"), &ctx)
            .await;

        // Then the session title was set to the first line of the message.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.title(), Some("my new chat"));
        drop(state);

        // Suppress unused variable warning.
        let _ = sink;
    }

    #[tokio::test]
    async fn dispatch_user_message_transitions_to_sending() {
        // Given a session in Idle phase.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When dispatching a user message.
        actor
            .dispatch_user_message(&session_id, &ChatEntry::user("hello"), &ctx)
            .await;

        // Then the session is in Sending phase.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(matches!(session.phase(), PhaseKind::Sending));
    }

    #[tokio::test]
    async fn dispatch_tool_continuation_emits_send_to_llm_provider() {
        // Given a session in Idle phase with history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("previous message"));
            state.session.active_session_id().clone()
        };

        // When dispatching a tool continuation.
        actor.dispatch_tool_continuation(&session_id, &ctx).await;

        // Then SendToLlmProvider was emitted with the history.
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(has_send, "expected SendToLlmProvider command");
    }

    #[tokio::test]
    async fn dispatch_user_message_provider_id_is_none_when_no_provider() {
        // Given a session with the default model (NO_PROVIDER_ID).
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When dispatching a user message.
        actor
            .dispatch_user_message(&session_id, &ChatEntry::user("hello"), &ctx)
            .await;

        // Then SendToLlmProvider has provider_id = None.
        let commands = sink.commands();
        let provider_id: Option<String> = commands.iter().find_map(|c| match c {
            Command::SendToLlmProvider(cmd) => cmd.provider_id.clone(),
            _ => None,
        });
        assert_eq!(
            provider_id, None,
            "expected provider_id None for NO_PROVIDER_ID"
        );
    }

    #[tokio::test]
    async fn dispatch_user_message_provider_id_is_some_when_model_set() {
        // Given a session with an explicit model.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state.active_session_mut().set_model("my-model".to_owned());
            state.session.active_session_id().clone()
        };

        // When dispatching a user message.
        actor
            .dispatch_user_message(&session_id, &ChatEntry::user("hello"), &ctx)
            .await;

        // Then SendToLlmProvider has provider_id = Some("my-model").
        let commands = sink.commands();
        let provider_id: Option<String> = commands.iter().find_map(|c| match c {
            Command::SendToLlmProvider(cmd) => cmd.provider_id.clone(),
            _ => None,
        });
        assert_eq!(
            provider_id,
            Some("my-model".to_owned()),
            "expected provider_id Some(\"my-model\")"
        );
    }

    #[tokio::test]
    async fn dispatch_tool_continuation_provider_id_is_none_when_no_provider() {
        // Given a session with the default model (NO_PROVIDER_ID) and history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("previous message"));
            state.session.active_session_id().clone()
        };

        // When dispatching a tool continuation.
        actor.dispatch_tool_continuation(&session_id, &ctx).await;

        // Then SendToLlmProvider has provider_id = None.
        let commands = sink.commands();
        let provider_id: Option<String> = commands.iter().find_map(|c| match c {
            Command::SendToLlmProvider(cmd) => cmd.provider_id.clone(),
            _ => None,
        });
        assert_eq!(
            provider_id, None,
            "expected provider_id None for NO_PROVIDER_ID in tool continuation"
        );
    }

    #[tokio::test]
    async fn dispatch_tool_continuation_provider_id_is_some_when_model_set() {
        // Given a session with an explicit model and history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("previous message"));
            state
                .active_session_mut()
                .set_model("tool-model".to_owned());
            state.session.active_session_id().clone()
        };

        // When dispatching a tool continuation.
        actor.dispatch_tool_continuation(&session_id, &ctx).await;

        // Then SendToLlmProvider has provider_id = Some("tool-model").
        let commands = sink.commands();
        let provider_id: Option<String> = commands.iter().find_map(|c| match c {
            Command::SendToLlmProvider(cmd) => cmd.provider_id.clone(),
            _ => None,
        });
        assert_eq!(
            provider_id,
            Some("tool-model".to_owned()),
            "expected provider_id Some(\"tool-model\") in tool continuation"
        );
    }

    #[tokio::test]
    async fn handle_processes_session_phase_changed_event() {
        // Given a session with a queued user message.
        let mut actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user(
                    "via handle",
                ))));
            state.session.active_session_id().clone()
        };

        // When calling handle with an ActorEnvelope::Event(SessionPhaseChanged).
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Sending,
            new_phase: PhaseKind::Idle,
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::SessionPhaseChanged(payload)),
                &ctx,
            )
            .await;

        // Then SendToLlmProvider was emitted (handle dispatched the queued item).
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            has_send,
            "expected SendToLlmProvider when handle processes SessionPhaseChanged"
        );
    }

    #[tokio::test]
    async fn dispatch_user_message_drains_steering_buffer_before_assembly() {
        // Given a session with a non-empty steering buffer.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session
                .steering_buffer_mut()
                .push_fragment("steer here".to_owned());
            state.session.active_session_id().clone()
        };

        // When dispatching a user message.
        actor
            .dispatch_user_message(&session_id, &ChatEntry::user("hello"), &ctx)
            .await;

        // Then the steering buffer is drained before assembly.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert!(
            session.steering_buffer().is_empty(),
            "steering buffer must be drained during dispatch_user_message"
        );

        // And the drained steering entry appears in history.
        let has_steering_entry = session.history().iter().any(|e| {
            matches!(
                &e.kind,
                crate::protocol::ChatEntryKind::User { expanded, .. } if expanded == "steer here"
            )
        });
        assert!(
            has_steering_entry,
            "drained steering entry must appear in history after dispatch_user_message"
        );
    }

    #[tokio::test]
    async fn dispatch_resume_drains_steering_buffer_before_assembly() {
        // Given a session with a non-empty steering buffer.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session
                .steering_buffer_mut()
                .push_fragment("resume steer".to_owned());
            state.session.active_session_id().clone()
        };

        // When dispatching a resume.
        actor.dispatch_resume(&session_id, &ctx, "test").await;

        // Then the steering buffer is drained before assembly.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        assert!(
            session.steering_buffer().is_empty(),
            "steering buffer must be drained during dispatch_resume"
        );

        // And the drained steering entry appears in history.
        let has_steering_entry = session.history().iter().any(|e| {
            matches!(
                &e.kind,
                crate::protocol::ChatEntryKind::User { expanded, .. } if expanded == "resume steer"
            )
        });
        assert!(
            has_steering_entry,
            "drained steering entry must appear in history after dispatch_resume"
        );
    }

    #[tokio::test]
    async fn idle_transition_with_steering_buffer_and_empty_queue_dispatches_resume() {
        // Given a session with steering fragments in the buffer and an empty queue.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session
                .steering_buffer_mut()
                .push_fragment("stay focused".to_owned());
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Idle phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        };
        actor.handle_session_phase_changed(&payload, &ctx).await;

        // Then SendToLlmProvider was emitted (resume dispatch ran).
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            has_send,
            "expected SendToLlmProvider for steering buffer resume dispatch"
        );

        // And the steering buffer is empty.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            session.steering_buffer().is_empty(),
            "steering buffer must be drained after resume dispatch"
        );
    }

    #[tokio::test]
    async fn idle_transition_with_queued_message_and_steering_buffer_dispatches_queue_first() {
        // Given a session with both a queued user message and steering fragments.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user(
                "queued message",
            ))));
            session
                .steering_buffer_mut()
                .push_fragment("steer here".to_owned());
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Idle phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        };
        actor.handle_session_phase_changed(&payload, &ctx).await;

        // Then SendToLlmProvider was emitted.
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            has_send,
            "expected SendToLlmProvider for queued user message"
        );

        // And the queue is empty.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            session.queue().is_empty(),
            "queue must be empty after dispatch"
        );

        // And the steering buffer is drained by dispatch_user_message.
        assert!(
            session.steering_buffer().is_empty(),
            "steering buffer must be drained during queue dispatch"
        );
    }

    #[tokio::test]
    async fn steering_buffer_drain_produces_entry_in_history() {
        // Given a session with steering fragments and empty queue.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session
                .steering_buffer_mut()
                .push_fragment("late correction".to_owned());
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Idle phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        };
        actor.handle_session_phase_changed(&payload, &ctx).await;

        // Then history contains a User entry with the steering text.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let has_steering_entry = session.history().iter().any(|e| {
            matches!(
                &e.kind,
                crate::protocol::ChatEntryKind::User { expanded, .. } if expanded == "late correction"
            )
        });
        assert!(
            has_steering_entry,
            "drained steering entry must appear in history"
        );
    }

    #[tokio::test]
    async fn steering_buffer_dispatch_includes_entry_in_llm_context() {
        // Given a session with existing history and steering fragments.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("original question"));
            session
                .steering_buffer_mut()
                .push_fragment("follow-up steer".to_owned());
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Idle phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        };
        actor.handle_session_phase_changed(&payload, &ctx).await;

        // Then SendToLlmProvider was emitted, proving the steering entry
        // was assembled into the LLM prompt alongside existing history.
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            has_send,
            "SendToLlmProvider must be emitted with steering entry in context"
        );
    }
}
