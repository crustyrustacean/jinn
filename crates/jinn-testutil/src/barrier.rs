//! A barrier for tests that assert something did **not** happen.
//!
//! # The problem
//!
//! A test that publishes a message, sleeps for a fixed interval, and then
//! asserts that no message came out is making a bet on two things at once:
//! that the actor under test *would* have produced the message, and that the
//! sleep was long enough for it to arrive. Both bets are wrong in the same
//! direction on a loaded machine: the assertion runs before the actor has
//! even been scheduled, so it passes — **vacuously**. A green suite then
//! proves nothing about the behaviour it claims to check.
//!
//! # The barrier
//!
//! An actor's inbox is dispatched **in order** (trouper claims a batch of
//! messages and dispatches each claim inline, in FIFO order). So if the
//! actor under test handles a message published *after* the one under test,
//! then receiving that later message proves the earlier one has already
//! been processed.
//!
//! [`Barrier`] is that later message. The actor under test declares it as
//! a handled schema and answers it by publishing [`BarrierAck`]; the test
//! waits for the ack. When the ack arrives, the work under test is done,
//! and asserting that nothing else arrived is meaningful rather than
//! vacuous.
//!
//! # Example
//!
//! ```
//! use std::time::Duration;
//!
//! use jinn_testutil::barrier::{Barrier, BarrierAck};
//! use jinn_testutil::bus_harness::{await_recorded, Recorder, TestHarness};
//!
//! # #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, trouper::schema::Event)]
//! # struct TriggerEvent;
//! # impl jinn_slices::route::BusMessage for TriggerEvent {}
//! # async fn demo(
//! #     harness: &TestHarness,
//! #     acks: &Recorder<BarrierAck>,
//! #     dispatched: &Recorder<TriggerEvent>,
//! # ) {
//! // The actor under test handles `Barrier` and publishes `BarrierAck`.
//! harness.publish(TriggerEvent).await;
//!
//! // When the barrier is published after the trigger.
//! harness.publish(Barrier::new("after-trigger")).await;
//! let _acks = await_recorded::<BarrierAck>(&acks, 1, Duration::from_secs(5)).await;
//!
//! // Then the trigger's absence is a real observation, not a race.
//! assert!(dispatched.is_empty());
//! # }
//! ```

use std::time::Duration;

use serde::{Deserialize, Serialize};

use jinn_slices::route::BusMessage;

/// A message the actor under test handles to acknowledge that it has
/// drained everything published before it.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A barrier request: acknowledge that earlier work is done.")]
pub struct Barrier {
    /// A caller-chosen label, so a test can tell two barriers apart.
    pub label: String,
}

impl Barrier {
    /// Creates a barrier with a caller-chosen label.
    #[must_use]
    pub fn new<L>(label: L) -> Self
    where
        L: Into<String>,
    {
        Self {
            label: label.into(),
        }
    }
}

/// The actor's answer to a [`Barrier`]: it has drained everything published
/// before the barrier.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A barrier acknowledgement: earlier work has been processed.")]
pub struct BarrierAck {
    /// The label of the barrier being acknowledged.
    pub label: String,
}

impl BusMessage for Barrier {}

impl BusMessage for BarrierAck {}

/// How long to wait for a barrier acknowledgement before giving up.
///
/// A barrier that never comes back means the actor under test never
/// processed the work — which is itself a failure, and one the caller must
/// hear about rather than paper over with a longer sleep.
pub const DEFAULT_BARRIER_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        reason = "test code: a failed expect aborts the test loudly, which is the desired mode"
    )]

    use std::sync::Arc;
    use std::time::Duration;

    use jinn_slices::route::BusMessage;
    use parking_lot::Mutex as ParkingMutex;
    use serde::{Deserialize, Serialize};
    use trouper::actor::{ActorPath, MsgHandler, ServiceActor};

    use super::Barrier;

    /// A message the actor under test handles *before* the barrier.
    #[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
    #[schema(description = "Test-only event handled before the barrier.")]
    struct Work {
        id: u32,
    }

    impl BusMessage for Work {}

    /// A recording actor that acknowledges a [`Barrier`] and remembers the
    /// work it saw before it.
    struct BarrierActor {
        seen: Arc<ParkingMutex<Vec<u32>>>,
        acks: Arc<ParkingMutex<Vec<String>>>,
    }

    impl ServiceActor for BarrierActor {
        /// Always an error: the actor spawns via `init_with`, so the
        /// plain `start` path is never the one taken. This is a hard
        /// invariant, not a stub.
        async fn init(
            _args: &trouper::json::Json,
        ) -> Result<Self, error_stack::Report<trouper::registry::RegistryError>> {
            std::future::ready(Err(error_stack::IntoReport::into_report(
                trouper::registry::RegistryError::InvalidSpec,
            )
            .attach("BarrierActor is spawned via init_with")))
            .await
        }
    }

    impl MsgHandler<Work> for BarrierActor {
        async fn handle(&mut self, msg: &Work, _ctx: &mut trouper::context::MsgCtx<'_>) {
            self.seen.lock().push(msg.id);
        }
    }

    impl MsgHandler<Barrier> for BarrierActor {
        async fn handle(&mut self, msg: &Barrier, _ctx: &mut trouper::context::MsgCtx<'_>) {
            self.acks.lock().push(msg.label.clone());
        }
    }

    /// The handles a spawned barrier actor exposes: its path, the work it
    /// saw, and the barriers it acknowledged.
    type BarrierHandles = (
        ActorPath,
        Arc<ParkingMutex<Vec<u32>>>,
        Arc<ParkingMutex<Vec<String>>>,
    );

    /// Spawns the barrier actor and returns its recording handles.
    fn spawn_actor(system: &trouper::system::ActorSystem) -> BarrierHandles {
        let seen = Arc::new(ParkingMutex::new(Vec::new()));
        let acks = Arc::new(ParkingMutex::new(Vec::new()));
        let path = ActorPath::new("test.barrier.actor");
        let seen_for_actor = seen.clone();
        let acks_for_actor = acks.clone();
        trouper::builder::spawn_service_builder::<BarrierActor>(system)
            .at(path.clone())
            .init_with(move || {
                let seen = seen_for_actor.clone();
                let acks = acks_for_actor.clone();
                Box::pin(async move { Ok(BarrierActor { seen, acks }) })
            })
            .handles::<Work>()
            .handles::<Barrier>()
            .start();
        (path, seen, acks)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn barrier_ack_arrives_after_the_work_before_it() {
        // Given an actor that records the work it handles and acknowledges barriers.
        let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
        let (_path, seen, acks) = spawn_actor(&system);

        // When work is published, then a barrier after it.
        system.publish(Work { id: 7 }).await;
        system.publish(Barrier::new("after-work")).await;

        // Then the barrier is acknowledged.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while acks.lock().is_empty() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(acks.lock().as_slice(), ["after-work"]);

        // And the work published before the barrier has already been handled.
        assert_eq!(seen.lock().as_slice(), [7]);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn several_barriers_are_acknowledged_in_order() {
        // Given an actor that acknowledges barriers.
        let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
        let (_path, seen, acks) = spawn_actor(&system);

        // When several barriers are published in sequence.
        system.publish(Barrier::new("first")).await;
        system.publish(Work { id: 1 }).await;
        system.publish(Barrier::new("second")).await;

        // Then they are acknowledged in order.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while acks.lock().len() < 2 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(acks.lock().as_slice(), ["first", "second"]);

        // And the work between them was handled before the second barrier.
        assert_eq!(seen.lock().as_slice(), [1]);
    }
}
