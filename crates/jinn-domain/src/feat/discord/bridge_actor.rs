//! The Discord bridge actor — a thin kameo actor that subscribes to jinn's bus
//! and forwards the events the bot cares about onto a bounded channel.
//!
//! # Why this exists
//!
//! Bus subscription requires a kameo actor handle (the bus hands events to an
//! `ActorRef`'s recipient). The poise gateway, by contrast, runs as a plain
//! tokio task owning its own websocket. This actor is the bridge between those
//! two worlds: it is the *only* thing that subscribes, and it just forwards.
//!
//! # What it forwards
//!
//! - [`SessionPhaseChanged`] with `new_phase == Idle` → [`BridgeEvent::TurnFinished`]
//! - [`SessionSetupCompleted`] → [`BridgeEvent::SetupCompleted`]
//!
//! All other bus traffic is ignored. The bot never sees streaming tokens or
//! intermediate tool calls — it only acts on turn boundaries and setup results.

use kameo::prelude::{Actor, ActorRef, Context, Message};
use tokio::sync::mpsc;

use crate::common::actor_deps::ActorDeps;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::session_lifecycle::protocol::event::SessionSetupCompleted;

use super::protocol::BridgeEvent;

/// The bridge actor.
///
/// Holds a clone of the bus subscription handle (`ActorDeps`) and the sender
/// half of the channel the gateway task drains. Stateless beyond that — every
/// reaction is a pure forward.
pub struct DiscordBridgeActor {
    /// Forwards bus events onto this channel as [`BridgeEvent`]s.
    tx: mpsc::Sender<BridgeEvent>,
}

/// Dependencies for [`DiscordBridgeActor`].
#[derive(Clone)]
pub struct DiscordBridgeActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
    /// Sender half of the bounded (64) bridge channel.
    pub tx: mpsc::Sender<BridgeEvent>,
}

impl Actor for DiscordBridgeActor {
    type Args = DiscordBridgeActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionPhaseChanged>())
            .await;
        args.deps
            .subscribe(actor_ref.recipient::<SessionSetupCompleted>())
            .await;

        Ok(Self { tx: args.tx })
    }
}

impl Message<SessionPhaseChanged> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionPhaseChanged, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_session_phase_changed(&msg);
    }
}

impl Message<SessionSetupCompleted> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionSetupCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_session_setup_completed(&msg);
    }
}

impl DiscordBridgeActor {
    /// Constructs an actor instance directly (for tests that bypass `on_start`).
    #[cfg(test)]
    pub(crate) fn new(tx: mpsc::Sender<BridgeEvent>) -> Self {
        Self { tx }
    }

    /// Forward phase changes to the gateway **only** when the new phase is
    /// `Idle`. Non-idle transitions (Streaming, Sending, …) are dropped.
    pub(super) fn handle_session_phase_changed(&self, payload: &SessionPhaseChanged) {
        if payload.new_phase != PhaseKind::Idle {
            return;
        }
        self.forward(BridgeEvent::TurnFinished {
            session_id: payload.session_id.clone(),
        });
    }

    /// Forward every setup completion (success or failure — the gateway
    /// formats the message from `cwd`/`error`).
    pub(super) fn handle_session_setup_completed(&self, payload: &SessionSetupCompleted) {
        self.forward(BridgeEvent::SetupCompleted {
            session_id: payload.session_id.clone(),
            cwd: payload.cwd.clone(),
            error: payload.error.clone(),
        });
    }

    /// Push one event onto the channel.
    ///
    /// A full channel means the gateway task is behind; rather than block the
    /// bus dispatch loop we drop with a warning. The next `Idle`/setup event
    /// will still arrive and trigger a fresh read from `State`.
    fn forward(&self, event: BridgeEvent) {
        if self.tx.try_send(event).is_err() {
            tracing::warn!("discord bridge channel full — event dropped");
        }
    }
}
