//! Observable behavior tests for the hydration worker pool.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use std::sync::Arc;
use std::time::Duration;

use jinn_core_types::SessionId;
use jinn_domain::common::bus::HarnessServices;
use jinn_session_state::SessionStoreService;
use jinn_testutil::bus_harness::{TestHarness, await_recorded};

use crate::hydrate::{HydrateCompleted, HydrateSession};
use crate::hydrate_worker::{
    HYDRATE_WORKER_POOL_SIZE, HydrateWorkerActor, HydrateWorkerActorDeps, hydrate_worker_path,
};
use crate::session_store_tests_support::ControlledStartupStore;

#[rstest::rstest]
#[tokio::test]
async fn dispatched_hydration_job_produces_a_completion() {
    // Given a spawned hydration pool and a persisted session.
    let persisted_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[(
        persisted_id.clone(),
        jiff::Timestamp::from_second(1).expect("valid timestamp"),
    )]));
    let harness = spawn_pool(store).await;
    let completed = harness.spawn_recorder::<HydrateCompleted>().await;

    // When a hydration job is dispatched for the session.
    dispatch_to_worker(&harness, persisted_id.clone(), false).await;

    // Then a completion carrying the snapshot comes back.
    let completed = await_recorded(&completed, 1, Duration::from_secs(1)).await;
    assert_eq!(completed[0].session_id, persisted_id);
    assert!(completed[0].snapshot.is_some());
}

#[rstest::rstest]
#[tokio::test]
async fn hydration_of_an_unknown_session_still_completes() {
    // Given a spawned hydration pool and a session the store does not hold.
    let store = Arc::new(ControlledStartupStore::new(&[]));
    let harness = spawn_pool(store).await;
    let completed = harness.spawn_recorder::<HydrateCompleted>().await;
    let missing_id = SessionId::new();

    // When a hydration job is dispatched for it.
    dispatch_to_worker(&harness, missing_id.clone(), false).await;

    // Then a completion still arrives, carrying no snapshot.
    let completed = await_recorded(&completed, 1, Duration::from_secs(1)).await;
    assert_eq!(completed[0].session_id, missing_id);
    assert!(completed[0].snapshot.is_none());
}

/// Spawns the pool against a controlled store, without the store actor.
async fn spawn_pool(store: Arc<ControlledStartupStore>) -> TestHarness {
    let harness = TestHarness::new().await;
    let mut services = harness.services().await;
    services.session_store = SessionStoreService::new(store);
    for index in 0..HYDRATE_WORKER_POOL_SIZE {
        HydrateWorkerActor::spawn(
            harness.system(),
            index,
            HydrateWorkerActorDeps {
                session_store: services.session_store.clone(),
            },
        );
    }
    harness
}

/// Sends one job to a fixed worker, so the test observes a known receiver.
async fn dispatch_to_worker(harness: &TestHarness, session_id: SessionId, frozen: bool) {
    harness
        .system()
        .tell(
            hydrate_worker_path(0),
            HydrateSession { session_id, frozen },
        )
        .await
        .expect("hydration job delivered");
}
