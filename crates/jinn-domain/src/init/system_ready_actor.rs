//! System-ready actor — signals the main thread when all actors have spawned.
//!
//! Subscribes to [`AllActorsSpawned`] events. When received, sends `()` on
//! a `oneshot::Sender` to unblock the main thread's `wait_for_system_ready` call.
//!
//! In the kameo system, all actors are spawned in `actor_wiring.rs`. After the
//! last actor is spawned, the wiring code publishes `AllActorsSpawned` to the bus.
//! This actor receives it and signals readiness.

use crate::common::actor::protocol::event::AllActorsSpawned;
use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
use kameo::prelude::{Actor, ActorRef, Context, Message, Spawn};

/// The system-ready actor.
///
/// Waits for `AllActorsSpawned` (confirming all actors have been spawned),
/// then sends `()` on the injected oneshot sender to wake the main thread.
pub struct SystemReadyActor {
    deps: ActorDeps,
    /// Oneshot sender to signal the main thread.
    ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Dependencies for [`SystemReadyActor`].
pub struct SystemReadyActorDeps {
    /// Universal actor dependencies.
    pub deps: ActorDeps,
    /// One-shot sender to signal system readiness to the main thread.
    pub ready_tx: tokio::sync::oneshot::Sender<()>,
}

impl Actor for SystemReadyActor {
    type Args = SystemReadyActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(
        args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<AllActorsSpawned>())
            .await;

        Ok(Self {
            deps: args.deps,
            ready_tx: Some(args.ready_tx),
        })
    }
}

impl Message<AllActorsSpawned> for SystemReadyActor {
    type Reply = ();

    async fn handle(&mut self, _msg: AllActorsSpawned, _ctx: &mut Context<Self, Self::Reply>) {
        tracing::info!("actor system ready — all actors spawned");
        if let Some(tx) = self.ready_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl BusPublish for SystemReadyActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
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
    use crate::common::bus::test_harness::TestHarness;

    #[tokio::test]
    async fn signals_on_all_actors_spawned() {
        // Given a SystemReadyActor.
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let harness = TestHarness::new().await;
        let actor = SystemReadyActor::spawn(SystemReadyActorDeps {
            deps: harness.actor_deps().await,
            ready_tx: tx,
        });
        actor.wait_for_startup().await;
        // When publishing AllActorsSpawned.
        harness.publish(AllActorsSpawned).await;

        // Then the oneshot is consumed.
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
        assert!(result.is_ok(), "oneshot should be sent");
    }

    #[tokio::test]
    async fn does_not_signal_without_all_actors_spawned() {
        // Given a SystemReadyActor.
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let harness = TestHarness::new().await;
        let _actor = SystemReadyActor::spawn(SystemReadyActorDeps {
            deps: harness.actor_deps().await,
            ready_tx: tx,
        });

        // When NOT publishing AllActorsSpawned.
        // Then the oneshot is NOT consumed.
        let result = rx.try_recv();
        assert!(result.is_err(), "oneshot should not be sent yet");
    }
}
