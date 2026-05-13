//! System-ready actor — counts `ActorStarted` events and signals the main thread.
//!
//! Subscribes to [`ActorStarted`] events and counts them against an injected
//! expected count (total actors in the system, including this one). When the
//! threshold is reached, sends `()` on a `oneshot::Sender` to unblock the
//! main thread's `wait_for_system_ready` call.

use crate::common::actor::protocol::event::ActorStarted;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg, SystemMessage};
use crate::protocol::Event;

/// The system-ready actor.
///
/// On each `ActorStarted` event, increments a counter. When the counter reaches
/// the expected count, sends `()` on the injected oneshot sender to wake the
/// main thread.
pub struct SystemReadyActor {
    /// Oneshot sender to signal the main thread.
    ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Number of `ActorStarted` events received so far.
    received: usize,
    /// Total number of actors in the system (including this one).
    expected: usize,
}

impl Actor for SystemReadyActor {
    type Message = NoDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "Data injection is required at activation"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<ActorStarted>();
        ctx.set_description("Counts ActorStarted events and signals system ready");

        let ready_tx = ctx
            .take_data::<tokio::sync::oneshot::Sender<()>>()
            .expect("SystemReadyActor requires oneshot::Sender injection");
        let expected = ctx
            .take_data::<usize>()
            .expect("SystemReadyActor requires expected actor count");

        Self {
            ready_tx: Some(ready_tx),
            received: 0,
            expected,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => {
                if let Event::ActorStarted { .. } = event {
                    self.received += 1;
                    if self.received >= self.expected {
                        tracing::info!(
                            received = self.received,
                            expected = self.expected,
                            "actor system ready"
                        );
                        if let Some(tx) = self.ready_tx.take() {
                            let _ = tx.send(());
                        }
                    }
                }
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Command(_) | ActorEnvelope::Shutdown => {}
        }
    }
}
