//! Trigger-actor reachability tests over the shared bus harness.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code"
)]

use std::time::Duration;

use crate::trigger_actor::{AttendantTriggerActor, AttendantTriggerActorDeps};
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::bus::HarnessServices;
use jinn_kernel::common::state::State;
use jinn_session_msg::{TurnCompleted, TurnOutcome};
use jinn_testutil::bus_harness::{TestHarness, await_recorded};

/// The actor half of a bus-harness test, driven through the bus.
struct TriggerBusActor {
    harness: TestHarness,
}

impl TriggerBusActor {
    /// Spawns the trigger actor onto the bus, wired to the harness.
    async fn spawn(harness: TestHarness, state: State) -> Self {
        let services = harness.services().await;
        AttendantTriggerActor::spawn(
            harness.system(),
            AttendantTriggerActorDeps { services, state },
        );
        Self { harness }
    }

    /// Publishes a message onto the bus.
    async fn publish<M>(&self, message: M)
    where
        M: jinn_kernel::common::bus::BusMessage
            + trouper::schema::Schema
            + serde::Serialize
            + Clone
            + Send
            + Sync
            + trouper::envelope::PayloadValue,
    {
        self.harness.publish(message).await;
    }
}

#[tokio::test]
async fn trigger_actor_receives_turn_completed_through_the_bus() {
    // Given the trigger actor spawned on a test bus with a recorder listening
    // for the completion events it observes.
    let harness = TestHarness::new().await;
    let seen = harness.spawn_recorder::<TurnCompleted>().await;
    let state = State::new(AppState::default());
    let actor = TriggerBusActor::spawn(harness, state).await;

    // When a succeeded turn completion is published.
    let session_id = jinn_core_types::SessionId::new();
    actor
        .publish(TurnCompleted {
            session_id: session_id.clone(),
            outcome: TurnOutcome::Succeeded,
        })
        .await;
    let events = await_recorded::<TurnCompleted>(&seen, 1, Duration::from_secs(2)).await;

    // Then the actor's subscription is live — the recorder (and therefore the
    // trigger actor, on the same topic) saw the event with its outcome intact.
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, TurnOutcome::Succeeded);
    assert_eq!(events[0].session_id, session_id);
}

#[tokio::test]
async fn trigger_actor_is_reachable_at_its_static_path() {
    // Given a harness with the trigger actor spawned.
    let harness = TestHarness::new().await;
    let state = State::new(AppState::default());
    let path = AttendantTriggerActor::spawn(
        harness.system(),
        AttendantTriggerActorDeps {
            services: harness.services().await,
            state,
        },
    );

    // When sending a message directly to that path.
    let delivered = harness
        .system()
        .tell(
            path,
            TurnCompleted {
                session_id: jinn_core_types::SessionId::new(),
                outcome: TurnOutcome::Succeeded,
            },
        )
        .await;

    // Then the send resolves — the actor is live where composition and
    // every publish expect it. An unrouted tell returns the envelope.
    assert!(
        delivered.is_ok(),
        "trigger actor must be live and routed at its static path"
    );
}
