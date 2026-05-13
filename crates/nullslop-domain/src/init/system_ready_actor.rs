//! System-ready actor — counts `ActorStarted` events and signals the main thread.
//!
//! Subscribes to [`ActorStarted`] and [`AllActorsSpawned`] events. Tracks the
//! running count of started actors. Only checks readiness after receiving
//! `AllActorsSpawned` (emitted by the wiring code after all actors are spawned).
//! When the received count matches the [`ActorCounter`] total, sends `()` on
//! a `oneshot::Sender` to unblock the main thread's `wait_for_system_ready` call.

use crate::common::actor::actor_counter::ActorCounter;
use crate::common::actor::protocol::event::{ActorStarted, AllActorsSpawned};
use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg, SystemMessage};
use crate::protocol::Event;

/// The system-ready actor.
///
/// Waits for `AllActorsSpawned` (confirming all actors have been spawned),
/// then checks if the running count of `ActorStarted` events matches the
/// expected total from the injected `ActorCounter`. When both conditions are
/// met, sends `()` on the injected oneshot sender to wake the main thread.
pub struct SystemReadyActor {
    /// Oneshot sender to signal the main thread.
    ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Number of `ActorStarted` events received so far.
    received: usize,
    /// Total number of actors in the system (shared counter).
    counter: ActorCounter,
    /// Whether `AllActorsSpawned` has been received.
    all_spawned: bool,
}

impl Actor for SystemReadyActor {
    type Message = NoDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "Data injection is required at activation"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<ActorStarted>();
        ctx.subscribe_event::<AllActorsSpawned>();
        ctx.set_description("Counts ActorStarted events and signals system ready");

        let ready_tx = ctx
            .take_data::<tokio::sync::oneshot::Sender<()>>()
            .expect("SystemReadyActor requires oneshot::Sender injection");
        let counter = ctx
            .take_data::<ActorCounter>()
            .expect("SystemReadyActor requires ActorCounter injection");

        Self {
            ready_tx: Some(ready_tx),
            received: 0,
            counter,
            all_spawned: false,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => {
                self.handle_event(event);
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Command(_) | ActorEnvelope::Shutdown => {}
        }
    }
}

impl SystemReadyActor {
    fn handle_event(&mut self, event: Event) {
        match event {
            Event::ActorStarted { .. } => {
                self.received += 1;
                self.maybe_signal_ready();
            }
            Event::AllActorsSpawned { .. } => {
                self.all_spawned = true;
                self.maybe_signal_ready();
            }
            _ => {}
        }
    }

    /// Signals the main thread if all actors have been spawned AND all have started.
    fn maybe_signal_ready(&mut self) {
        if !self.all_spawned {
            return;
        }
        let expected = self.counter.load() as usize;
        if self.received >= expected {
            tracing::info!(
                received = self.received,
                expected,
                "actor system ready"
            );
            if let Some(tx) = self.ready_tx.take() {
                let _ = tx.send(());
            }
        }
    }
}
