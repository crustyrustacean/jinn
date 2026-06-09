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

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let recipient = actor_ref.recipient::<SessionPhaseChanged>();
        args.bus
            .actor_ref()
            .tell(Register(recipient))
            .await
            .expect("queue actor failed to register for SessionPhaseChanged");

        Ok(Self {
            state: args.state,
            counter: args.counter,
            bus: args.bus,
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
mod tests;
