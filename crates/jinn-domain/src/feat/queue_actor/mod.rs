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

use kameo::prelude::{Actor, ActorRef, Context, Message};
use kameo_actors::message_bus::{Publish, Register};

use crate::common::services::bus_service::BusService;
use crate::common::state::State;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::context::assemble::assemble_prompt;
use crate::feat::context::strategy::token_estimator::TiktokenCounter;
use crate::feat::provider::protocol::command::SendToLlmProvider;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::session::queue_item::QueueItem;
use crate::feat::session_lifecycle::protocol::command::PersistSession;
use crate::protocol::SessionId;

/// The queue actor.
///
/// The sole consumer of the turn dispatch queue. Reacts to session phase
/// transitions to `Idle` by popping and dispatching queued items.
pub struct QueueActor {
    /// Shared application state (read/write access to session queue and data).
    state: State,
    /// Token counter for recording token usage in the session ledger.
    counter: TiktokenCounter,
    /// Reference to the message bus for publishing commands and events.
    bus: BusService,
}

/// Dependencies for [`QueueActor`].
pub struct QueueActorDeps {
    /// Shared application state.
    pub state: State,
    /// Token counter for usage tracking.
    pub counter: TiktokenCounter,
    /// Reference to the message bus for publishing commands and events.
    pub bus: BusService,
}

impl Actor for QueueActor {
    type Args = QueueActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(deps: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let recipient = actor_ref.recipient::<SessionPhaseChanged>();
        deps.bus
            .actor_ref()
            .tell(Register(recipient))
            .await
            .expect("queue actor failed to register for SessionPhaseChanged");

        Ok(Self {
            state: deps.state,
            counter: deps.counter,
            bus: deps.bus,
        })
    }
}

impl Message<SessionPhaseChanged> for QueueActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: SessionPhaseChanged,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        self.handle_session_phase_changed(&msg).await;
    }
}

impl QueueActor {
    /// Handle `SessionPhaseChanged` - dispatch on phase transitions.
    async fn handle_session_phase_changed(&self, payload: &SessionPhaseChanged) {
        match (payload.old_phase, payload.new_phase) {
            (_, PhaseKind::Idle) => {
                self.handle_idle_transition(&payload.session_id).await;
            }
            _ => {}
        }
    }

    /// Handle Idle transition - pop and dispatch the next queued item.
    async fn handle_idle_transition(&self, session_id: &SessionId) {
        let item = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            session.dequeue()
        };

        let Some(item) = item else { return };

        match item {
            QueueItem::UserMessage(entry) => {
                self.dispatch_user_message(session_id, &entry).await;
            }
            QueueItem::ToolContinuation => {
                self.dispatch_tool_continuation(session_id).await;
            }
        }
    }

    /// Dispatch a user message: push to history, set title, begin sending,
    /// assemble prompt, emit SendToLlmProvider, emit ChatEntrySubmitted, emit PersistSession.
    #[expect(clippy::unused_async, reason = "trait contract requires async")]
    async fn dispatch_user_message(
        &self,
        session_id: &SessionId,
        entry: &crate::protocol::ChatEntry,
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

        let bus_ref = self.bus.actor_ref();
        if let Err(e) = bus_ref
            .tell(Publish(SendToLlmProvider {
                session_id: session_id.clone(),
                messages: assembled.messages,
                provider_id,
                estimated_tokens,
                tool_definitions: assembled.tool_definitions,
            }))
            .await
        {
            tracing::warn!(err = ?e, "queue-actor failed to emit SendToLlmProvider");
        }

        if let Err(e) = bus_ref
            .tell(Publish(ChatEntrySubmitted {
                session_id: session_id.clone(),
                entry: entry.clone(),
            }))
            .await
        {
            tracing::warn!(err = ?e, "queue-actor failed to emit ChatEntrySubmitted");
        }

        if let Err(e) = bus_ref
            .tell(Publish(PersistSession {
                session_id: session_id.clone(),
            }))
            .await
        {
            tracing::warn!(err = ?e, "queue-actor failed to emit PersistSession");
        }
    }

    /// Dispatch a tool continuation: assemble prompt and emit SendToLlmProvider.
    ///
    /// See [`Self::dispatch_resume`] for the shared dispatch body.
    async fn dispatch_tool_continuation(&self, session_id: &SessionId) {
        self.dispatch_resume(session_id, "tool continuation").await;
    }

    /// Shared dispatch body for tool-continuation and manual-resume paths:
    /// re-assemble prompt from current history and emit `SendToLlmProvider`.
    #[expect(clippy::unused_async, reason = "trait contract requires async")]
    async fn dispatch_resume(&self, session_id: &SessionId, label: &str) {
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

        let bus_ref = self.bus.actor_ref();
        if let Err(e) = bus_ref
            .tell(Publish(SendToLlmProvider {
                session_id: session_id.clone(),
                messages: assembled.messages,
                provider_id,
                estimated_tokens,
                tool_definitions: assembled.tool_definitions,
            }))
            .await
        {
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

    use std::time::Duration;

    use kameo::actor::Spawn;
    use kameo_actors::message_bus::MessageBus;

    use super::*;
    use crate::common::app_state::AppState;
    use crate::protocol::ChatEntry;

    /// A simple recorder actor that collects messages of type M.
    pub struct Recorder<M> {
        messages: Vec<M>,
    }

    impl<M: Send + 'static> Actor for Recorder<M> {
        type Args = ();
        type Error = std::convert::Infallible;

        async fn on_start(
            _args: Self::Args,
            _actor_ref: ActorRef<Self>,
        ) -> Result<Self, Self::Error> {
            Ok(Self { messages: vec![] })
        }
    }

    /// Query message to retrieve collected messages from a Recorder.
    pub struct GetRecorded;

    impl<M: Clone + Send + 'static> Message<GetRecorded> for Recorder<M> {
        type Reply = Vec<M>;

        async fn handle(
            &mut self,
            _msg: GetRecorded,
            _ctx: &mut Context<Self, Self::Reply>,
        ) -> Self::Reply {
            self.messages.clone()
        }
    }

    impl<M: Clone + Send + 'static + 'static> Message<M> for Recorder<M> {
        type Reply = ();

        async fn handle(&mut self, msg: M, _ctx: &mut Context<Self, Self::Reply>) {
            self.messages.push(msg);
        }
    }

    fn test_state() -> State {
        State::new(AppState::default())
    }

    fn test_counter() -> TiktokenCounter {
        TiktokenCounter::o200k_base()
    }

    async fn setup() -> (
        ActorRef<QueueActor>,
        ActorRef<Recorder<SendToLlmProvider>>,
        ActorRef<Recorder<ChatEntrySubmitted>>,
        ActorRef<Recorder<PersistSession>>,
        kameo::prelude::ActorRef<MessageBus>,
        State,
    ) {
        let bus_ref = kameo::actor::Spawn::spawn(MessageBus::new(
            kameo_actors::DeliveryStrategy::BestEffort,
        ));
        let bus_service = BusService::new(bus_ref.clone());

        let state = test_state();
        let counter = test_counter();

        let queue = QueueActor::spawn(QueueActorDeps {
            state: state.clone(),
            counter,
            bus: bus_service,
        });

        let send_recorder = Recorder::<SendToLlmProvider>::spawn(());
        let chat_recorder = Recorder::<ChatEntrySubmitted>::spawn(());
        let persist_recorder = Recorder::<PersistSession>::spawn(());

        bus_ref
            .tell(Register(send_recorder.clone().recipient::<SendToLlmProvider>()))
            .await
            .expect("register SendToLlmProvider recorder");
        bus_ref
            .tell(Register(chat_recorder.clone().recipient::<ChatEntrySubmitted>()))
            .await
            .expect("register ChatEntrySubmitted recorder");
        bus_ref
            .tell(Register(
                persist_recorder.clone().recipient::<PersistSession>(),
            ))
            .await
            .expect("register PersistSession recorder");

        // Give the bus time to process registrations.
        tokio::time::sleep(Duration::from_millis(50)).await;

        (
            queue,
            send_recorder,
            chat_recorder,
            persist_recorder,
            bus_ref,
            state,
        )
    }

    #[tokio::test]
    async fn session_phase_changed_idle_pops_user_message_from_queue() {
        // Given a session with a queued user message in Idle phase.
        let (_queue, send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user(
                "queued message",
            ))));
            s.session.active_session_id().clone()
        };

        // When publishing a SessionPhaseChanged with Idle phase.
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then SendToLlmProvider was emitted for the queued message.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let send_cmds: Vec<SendToLlmProvider> =
            send_recorder.ask(GetRecorded).await.expect("get recorded");
        assert!(
            !send_cmds.is_empty(),
            "expected SendToLlmProvider command for queued user message"
        );

        // And the queue is empty.
        let s = state.read();
        let session = s.session.get(&session_id).expect("session exists");
        assert!(
            session.queue_len() == 0,
            "expected queue to be empty after dispatch"
        );
    }

    #[tokio::test]
    async fn session_phase_changed_non_idle_does_not_pop_queue() {
        // Given a session with a queued user message in non-Idle phase.
        let (_queue, send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user(
                "queued message",
            ))));
            session.begin_sending();
            s.session.active_session_id().clone()
        };

        // When publishing a SessionPhaseChanged with Sending phase.
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Streaming,
                new_phase: PhaseKind::Sending,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then no SendToLlmProvider was emitted.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let send_cmds: Vec<SendToLlmProvider> =
            send_recorder.ask(GetRecorded).await.expect("get recorded");
        assert!(
            send_cmds.is_empty(),
            "expected no SendToLlmProvider for non-Idle phase"
        );

        // And the queue still has the item.
        let s = state.read();
        let session = s.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 1);
    }

    #[tokio::test]
    async fn session_phase_changed_idle_with_empty_queue_is_noop() {
        // Given a session with an empty queue.
        let (_queue, send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let s = state.read();
            s.session.active_session_id().clone()
        };

        // When publishing a SessionPhaseChanged with Idle phase.
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Streaming,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then no commands were emitted.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let send_cmds: Vec<SendToLlmProvider> =
            send_recorder.ask(GetRecorded).await.expect("get recorded");
        assert!(send_cmds.is_empty(), "expected no commands for empty queue");
    }

    #[tokio::test]
    async fn dispatch_user_message_emits_chat_entry_submitted() {
        // Given a queue actor wired to the bus.
        let (_queue, _send_recorder, chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let s = state.read();
            s.session.active_session_id().clone()
        };

        // When dispatching a user message by publishing an Idle transition.
        {
            let mut s = state.write();
            s.active_session_mut()
                .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
        }
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then ChatEntrySubmitted was emitted.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let chat_events: Vec<ChatEntrySubmitted> =
            chat_recorder.ask(GetRecorded).await.expect("get recorded");
        assert!(
            !chat_events.is_empty(),
            "expected ChatEntrySubmitted event"
        );
    }

    #[tokio::test]
    async fn dispatch_user_message_sets_title_on_first_message() {
        // Given a session with no title.
        let (_queue, _send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let s = state.read();
            s.session.active_session_id().clone()
        };

        // When dispatching a user message.
        {
            let mut s = state.write();
            s.active_session_mut()
                .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("my new chat"))));
        }
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then the session title was set to the first line of the message.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let s = state.read();
        let session = s.session.get(&session_id).expect("session exists");
        assert_eq!(session.title(), Some("my new chat"));
    }

    #[tokio::test]
    async fn dispatch_user_message_transitions_to_sending() {
        // Given a session in Idle phase.
        let (_queue, _send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let s = state.read();
            s.session.active_session_id().clone()
        };

        // When dispatching a user message.
        {
            let mut s = state.write();
            s.active_session_mut()
                .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
        }
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then the session is in Sending phase.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let s = state.read();
        let session = s.session.get(&session_id).expect("session exists");
        assert!(matches!(session.phase(), PhaseKind::Sending));
    }

    #[tokio::test]
    async fn dispatch_tool_continuation_emits_send_to_llm_provider() {
        // Given a session in Idle phase with history and a tool continuation queued.
        let (_queue, send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.push_entry(ChatEntry::user("previous message"));
            session.enqueue(QueueItem::ToolContinuation);
            s.session.active_session_id().clone()
        };

        // When dispatching a tool continuation via Idle transition.
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then SendToLlmProvider was emitted.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let send_cmds: Vec<SendToLlmProvider> =
            send_recorder.ask(GetRecorded).await.expect("get recorded");
        assert!(!send_cmds.is_empty(), "expected SendToLlmProvider command");
    }

    #[tokio::test]
    async fn dispatch_user_message_provider_id_is_none_when_no_provider() {
        // Given a session with the default model (NO_PROVIDER_ID).
        let (_queue, send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let s = state.read();
            s.session.active_session_id().clone()
        };

        // When dispatching a user message.
        {
            let mut s = state.write();
            s.active_session_mut()
                .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
        }
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then SendToLlmProvider has provider_id = None.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let send_cmds: Vec<SendToLlmProvider> =
            send_recorder.ask(GetRecorded).await.expect("get recorded");
        let provider_id = send_cmds.first().map(|c| c.provider_id.clone());
        assert_eq!(
            provider_id,
            Some(None),
            "expected provider_id None for NO_PROVIDER_ID"
        );
    }

    #[tokio::test]
    async fn dispatch_user_message_provider_id_is_some_when_model_set() {
        // Given a session with an explicit model.
        let (_queue, send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let mut s = state.write();
            s.active_session_mut().set_model("my-model".to_owned());
            s.active_session_mut()
                .enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
            s.session.active_session_id().clone()
        };

        // When dispatching a user message.
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then SendToLlmProvider has provider_id = Some("my-model").
        tokio::time::sleep(Duration::from_millis(100)).await;
        let send_cmds: Vec<SendToLlmProvider> =
            send_recorder.ask(GetRecorded).await.expect("get recorded");
        let provider_id = send_cmds.first().map(|c| c.provider_id.clone());
        assert_eq!(
            provider_id,
            Some(Some("my-model".to_owned())),
            "expected provider_id Some(\"my-model\")"
        );
    }

    #[tokio::test]
    async fn dispatch_tool_continuation_provider_id_is_none_when_no_provider() {
        // Given a session with the default model (NO_PROVIDER_ID) and history.
        let (_queue, send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.push_entry(ChatEntry::user("previous message"));
            session.enqueue(QueueItem::ToolContinuation);
            s.session.active_session_id().clone()
        };

        // When dispatching a tool continuation.
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then SendToLlmProvider has provider_id = None.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let send_cmds: Vec<SendToLlmProvider> =
            send_recorder.ask(GetRecorded).await.expect("get recorded");
        let provider_id = send_cmds.first().map(|c| c.provider_id.clone());
        assert_eq!(
            provider_id,
            Some(None),
            "expected provider_id None for NO_PROVIDER_ID in tool continuation"
        );
    }

    #[tokio::test]
    async fn dispatch_tool_continuation_provider_id_is_some_when_model_set() {
        // Given a session with an explicit model and history.
        let (_queue, send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.push_entry(ChatEntry::user("previous message"));
            session.enqueue(QueueItem::ToolContinuation);
            s.active_session_mut()
                .set_model("tool-model".to_owned());
            s.session.active_session_id().clone()
        };

        // When dispatching a tool continuation.
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then SendToLlmProvider has provider_id = Some("tool-model").
        tokio::time::sleep(Duration::from_millis(100)).await;
        let send_cmds: Vec<SendToLlmProvider> =
            send_recorder.ask(GetRecorded).await.expect("get recorded");
        let provider_id = send_cmds.first().map(|c| c.provider_id.clone());
        assert_eq!(
            provider_id,
            Some(Some("tool-model".to_owned())),
            "expected provider_id Some(\"tool-model\") in tool continuation"
        );
    }

    #[tokio::test]
    async fn dispatch_user_message_drains_steering_buffer_before_assembly() {
        // Given a session with a non-empty steering buffer and a queued user message.
        let (_queue, _send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session
                .steering_buffer_mut()
                .push_fragment("steer here".to_owned());
            session.enqueue(QueueItem::UserMessage(Box::new(ChatEntry::user("hello"))));
            s.session.active_session_id().clone()
        };

        // When dispatching a user message via Idle transition.
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then the steering buffer is drained before assembly.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let s = state.read();
        let session = s.session.get(&session_id).expect("session");
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
        // Given a session with a non-empty steering buffer and a tool continuation queued.
        let (_queue, _send_recorder, _chat_recorder, _persist_recorder, bus_ref, state) =
            setup().await;
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.push_entry(ChatEntry::user("previous"));
            session
                .steering_buffer_mut()
                .push_fragment("resume steer".to_owned());
            session.enqueue(QueueItem::ToolContinuation);
            s.session.active_session_id().clone()
        };

        // When dispatching a resume via Idle transition.
        bus_ref
            .tell(Publish(SessionPhaseChanged {
                session_id: session_id.clone(),
                old_phase: PhaseKind::Sending,
                new_phase: PhaseKind::Idle,
            }))
            .await
            .expect("publish SessionPhaseChanged");

        // Then the steering buffer is drained before assembly.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let s = state.read();
        let session = s.session.get(&session_id).expect("session");
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
}
