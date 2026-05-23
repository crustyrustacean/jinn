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

//! Queue actor — sole owner of turn dispatch queue consumption.
//!
//! Subscribes to [`SessionPhaseChanged`] events and [`EnqueueCompaction`] commands.
//! When a session transitions to `Idle`, pops the next item from the turn queue
//! and dispatches it. When [`EnqueueCompaction`] arrives and the session is idle,
//! dispatches compaction immediately; otherwise enqueues for later.
//!
//! # Dispatch behavior
//!
//! - `UserMessage` → push entry, set title, begin sending, call `assemble_prompt()` + emit `SendToLlmProvider`
//! - `ToolContinuation` → call `assemble_prompt()` + emit `SendToLlmProvider`
//! - `CompactionNeeded` → emit `CompactContext`

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::compaction_actor::protocol::command::{CompactContext, EnqueueCompaction};
use crate::feat::context::assemble::assemble_prompt;
use crate::feat::context::strategy::token_estimator::TiktokenCounter;
use crate::feat::provider::protocol::command::SendToLlmProvider;
use crate::feat::session::chat_session::SessionPhase;
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
        ctx.subscribe_command::<EnqueueCompaction>();

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
            ActorEnvelope::Command(Command::EnqueueCompaction(payload)) => {
                self.handle_enqueue_compaction(&payload, ctx).await;
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::System(_)
            | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl QueueActor {
    /// Handle `SessionPhaseChanged` — pop queue on Idle transition.
    async fn handle_session_phase_changed(
        &self,
        payload: &SessionPhaseChanged,
        ctx: &ActorContext,
    ) {
        if payload.new_phase != SessionPhase::Idle {
            return;
        }

        let item = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.dequeue()
        };

        let Some(item) = item else { return };

        match item {
            QueueItem::UserMessage(entry) => {
                self.dispatch_user_message(&payload.session_id, &entry, ctx)
                    .await;
            }
            QueueItem::ToolContinuation => {
                self.dispatch_tool_continuation(&payload.session_id, ctx)
                    .await;
            }
            QueueItem::CompactionNeeded => {
                self.dispatch_compaction(&payload.session_id, ctx).await;
            }
        }
    }

    /// Handle `EnqueueCompaction` — if idle, dispatch immediately; otherwise enqueue.
    async fn handle_enqueue_compaction(&self, payload: &EnqueueCompaction, ctx: &ActorContext) {
        let is_idle = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            if matches!(session.phase(), SessionPhase::Idle) {
                true
            } else {
                session.enqueue(QueueItem::CompactionNeeded);
                false
            }
        };

        if is_idle {
            self.dispatch_compaction(&payload.session_id, ctx).await;
        }
    }

    /// Dispatch a user message: push to history, set title, begin sending,
    /// assemble prompt, emit SendToLlmProvider, emit ChatEntrySubmitted, emit PersistSession.
    #[allow(clippy::unused_async)]
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
            session.begin_sending();
        }

        let assembled = {
            let guard = self.state.read();
            assemble_prompt(&guard, session_id, &self.counter)
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
    #[allow(clippy::unused_async)]
    async fn dispatch_tool_continuation(
        &self,
        session_id: &crate::protocol::SessionId,
        ctx: &ActorContext,
    ) {
        let assembled = {
            let guard = self.state.read();
            assemble_prompt(&guard, session_id, &self.counter)
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
        })) {
            tracing::warn!(
                err = ?e,
                "queue-actor failed to emit SendToLlmProvider from tool continuation"
            );
        }
    }

    /// Dispatch compaction: emit CompactContext if session is Idle.
    #[allow(clippy::unused_async)]
    async fn dispatch_compaction(
        &self,
        session_id: &crate::protocol::SessionId,
        ctx: &ActorContext,
    ) {
        let phase = {
            let state = self.state.read();
            state.session(session_id).phase()
        };

        if !matches!(phase, SessionPhase::Idle) {
            tracing::warn!(
                session_id = ?session_id,
                current_phase = ?phase,
                "CompactionNeeded dispatched but session is not Idle — skipping"
            );
            return;
        }

        if let Err(e) = ctx.send_command(Command::CompactContext(CompactContext {
            session_id: session_id.clone(),
        })) {
            tracing::warn!(err = ?e, "queue-actor failed to emit CompactContext");
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use crate::common::actor::{ActorContext, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::feat::session::chat_session::SessionPhase;
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
            session.enqueue(QueueItem::UserMessage(ChatEntry::user("queued message")));
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Idle phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            new_phase: SessionPhase::Idle,
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
    async fn session_phase_changed_idle_pops_compaction_needed_from_queue() {
        // Given a session with a queued CompactionNeeded item in Idle phase.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(QueueItem::CompactionNeeded);
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Idle phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            new_phase: SessionPhase::Idle,
        };
        actor.handle_session_phase_changed(&payload, &ctx).await;

        // Then CompactContext was emitted.
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(
            has_compact,
            "expected CompactContext command for queued CompactionNeeded"
        );

        // And the queue is empty.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 0);
    }

    #[tokio::test]
    async fn session_phase_changed_non_idle_does_not_pop_queue() {
        // Given a session with a queued user message in non-Idle phase.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(QueueItem::UserMessage(ChatEntry::user("queued message")));
            session.begin_sending();
            state.session.active_session_id().clone()
        };

        // When handling SessionPhaseChanged with Sending phase.
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            new_phase: SessionPhase::Sending,
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
            new_phase: SessionPhase::Idle,
        };
        actor.handle_session_phase_changed(&payload, &ctx).await;

        // Then no commands were emitted.
        let commands = sink.commands();
        assert!(commands.is_empty(), "expected no commands for empty queue");
    }

    #[tokio::test]
    async fn enqueue_compaction_dispatches_immediately_when_idle() {
        // Given a session in Idle phase.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When handling EnqueueCompaction.
        let payload = EnqueueCompaction {
            session_id: session_id.clone(),
        };
        actor.handle_enqueue_compaction(&payload, &ctx).await;

        // Then CompactContext was emitted directly (not queued).
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(has_compact, "expected CompactContext for idle session");

        // And the queue is still empty (was never enqueued).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 0);
    }

    #[tokio::test]
    async fn enqueue_compaction_queues_when_not_idle() {
        // Given a session in Sending phase.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state.active_session_mut().begin_sending();
            state.session.active_session_id().clone()
        };

        // When handling EnqueueCompaction.
        let payload = EnqueueCompaction {
            session_id: session_id.clone(),
        };
        actor.handle_enqueue_compaction(&payload, &ctx).await;

        // Then no CompactContext was emitted.
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(!has_compact, "expected no CompactContext for busy session");

        // And the queue has the item.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 1);
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
        assert!(matches!(session.phase(), SessionPhase::Sending));
    }

    #[tokio::test]
    async fn dispatch_compaction_skips_when_not_idle() {
        // Given a session in Sending phase.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state.active_session_mut().begin_sending();
            state.session.active_session_id().clone()
        };

        // When dispatching compaction.
        actor.dispatch_compaction(&session_id, &ctx).await;

        // Then no CompactContext was emitted.
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(
            !has_compact,
            "expected no CompactContext for non-idle session"
        );
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
}
