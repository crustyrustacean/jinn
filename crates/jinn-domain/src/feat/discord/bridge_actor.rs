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

use crate::common::actor_deps::ActorDeps;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::session_archived::SessionArchived;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::session_lifecycle::protocol::event::{
    SessionSetupCompleted, SessionTeardownFinished,
};

use super::protocol::{BridgeEvent, CreateThreadForSession, GatewayRequest};

/// The bridge actor.
///
/// Holds a clone of the bus subscription handle (`ActorDeps`), the sender
/// half of the channel the gateway task drains, and a clone of [`State`] for
/// writing `ChatEntry` confirmation/error entries inline on result events.
pub struct DiscordBridgeActor {
    /// Forwards bus events onto this channel as [`BridgeEvent`]s.
    tx: kanal::Sender<BridgeEvent>,
    /// Forwards `CreateThreadForSession` bus commands onto this channel as
    /// [`GatewayRequest`]s — the reverse direction (domain → gateway do-something).
    gateway_tx: kanal::Sender<GatewayRequest>,
}

/// Dependencies for [`DiscordBridgeActor`].
#[derive(Clone)]
pub struct DiscordBridgeActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
    /// Sender half of the bounded (64) bridge channel.
    pub tx: kanal::Sender<BridgeEvent>,
    /// Sender half of the bounded (16) gateway-request channel.
    pub gateway_tx: kanal::Sender<GatewayRequest>,
}

impl Actor for DiscordBridgeActor {
    type Args = DiscordBridgeActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionPhaseChanged>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionSetupCompleted>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionTeardownFinished>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionArchived>())
            .await;
        args.deps
            .subscribe(actor_ref.recipient::<CreateThreadForSession>())
            .await;

        Ok(Self {
            tx: args.tx,
            gateway_tx: args.gateway_tx,
        })
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

impl Message<SessionTeardownFinished> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: SessionTeardownFinished,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        self.handle_session_teardown_finished(&msg);
    }
}

impl Message<SessionArchived> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionArchived, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_session_archived(&msg);
    }
}

impl Message<CreateThreadForSession> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(&mut self, msg: CreateThreadForSession, _ctx: &mut Context<Self, Self::Reply>) {
        self.forward_gateway_request(GatewayRequest::CreateThreadForSession {
            session_id: msg.session_id,
            title: msg.title,
        });
    }
}

impl DiscordBridgeActor {
    /// Constructs an actor instance directly (for tests that bypass `on_start`).
    #[cfg(test)]
    pub(crate) fn new(tx: kanal::Sender<BridgeEvent>) -> Self {
        let (gateway_tx, _gateway_rx) = kanal::bounded(1);
        Self { tx, gateway_tx }
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

    /// Forward every teardown completion (success or failure — the gateway
    /// formats the message from `error`).
    pub(super) fn handle_session_teardown_finished(&self, payload: &SessionTeardownFinished) {
        self.forward(BridgeEvent::TeardownFinished {
            session_id: payload.session_id.clone(),
            error: payload.error.clone(),
        });
    }

    /// Forward every archive completion to the gateway.
    pub(super) fn handle_session_archived(&self, payload: &SessionArchived) {
        self.forward(BridgeEvent::Archived {
            session_id: payload.session_id.clone(),
        });
    }

    /// Push one event onto the channel.
    ///
    /// A full channel means the gateway task is behind; rather than block the
    /// bus dispatch loop we drop with a warning. The next `Idle`/setup event
    /// will still arrive and trigger a fresh read from `State`.
    fn forward(&self, event: BridgeEvent) {
        tracing::info!(event = %event_discriminant(&event), "discord bridge forwarding");
        if !matches!(self.tx.try_send(event), Ok(true)) {
            tracing::warn!("discord bridge channel full — event dropped");
        }
    }

    /// Push one gateway request onto the request channel.
    ///
    /// Same drop-on-full semantics as [`forward`](Self::forward) — a full
    /// channel means the gateway task is behind, so we drop with a warning
    /// rather than block the bus dispatch loop.
    fn forward_gateway_request(&self, request: GatewayRequest) {
        tracing::info!("discord bridge forwarding gateway request");
        if !matches!(self.gateway_tx.try_send(request), Ok(true)) {
            tracing::warn!("discord gateway request channel full — request dropped");
        }
    }
}

/// Short label identifying a [`BridgeEvent`] variant for log lines.
///
/// The events themselves may carry large payloads (session ids are fine,
/// but keeping a single helper avoids per-arm `Display` requirements).
fn event_discriminant(event: &BridgeEvent) -> &'static str {
    match event {
        BridgeEvent::SetupCompleted { .. } => "SetupCompleted",
        BridgeEvent::TurnFinished { .. } => "TurnFinished",
        BridgeEvent::TeardownFinished { .. } => "TeardownFinished",
        BridgeEvent::Archived { .. } => "Archived",
    }
}
