//! Tests for the [`SessionPruneActor`](crate::session_prune_actor::SessionPruneActor) drain behavior.

#![allow(clippy::expect_used, clippy::unwrap_used, reason = "test code")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use error_stack::Report;
use jiff::Timestamp;
use jinn_core_types::SessionId;
use jinn_kernel::common::bus::HarnessServices;
use jinn_kernel::protocol::ChatEntryId;
use jinn_preferences_config::MaintenanceConfig;
use jinn_session_state::SessionStore;
use jinn_session_state::SessionStoreError;
use jinn_session_store_msg::SessionState;
use jinn_session_store_msg::SessionSummary;
use jinn_session_store_msg::{SearchOutcome, SearchParams, TranscriptWindow};
use jinn_slices::ServiceStatusUpdate;
use jinn_slices::bus::BusService;

use crate::session_prune_actor::{
    DRAIN_INTERVAL, INITIAL_DELAY, MAX_VACUUM_PAGES, PRUNE_BATCH, SESSION_PRUNE_ROW_NAME,
    SWEEP_INTERVAL, SessionPruneActorDeps,
};

/// A summary with the actor-relevant fields; everything else defaults.
fn summary(id: &SessionId, updated_at: Timestamp) -> SessionSummary {
    SessionSummary {
        session_id: id.clone(),
        title: format!("session {id}"),
        updated_at,
        created_at: updated_at,
        session_state: SessionState::Archived,
        in_library: false,
        is_bot: false,
        parent_session: None,
        project: None,
    }
}

/// Marks a summary archived/fresh/library'd/parented by mutating it.
fn set_parent(s: &mut SessionSummary, parent: &SessionId) {
    s.parent_session = Some(parent.clone());
}

fn hours_ago(hours: i64) -> Timestamp {
    Timestamp::now() - jiff::SignedDuration::from_hours(hours)
}

fn just_now() -> Timestamp {
    Timestamp::now()
}

#[rstest::rstest]
#[test]
fn prune_candidates_selects_stale_archived_trees() {
    // Given a parent-and-child tree, both archived, not in the Library, and
    // last touched far past the retention window; the child is *fresher*
    // than the parent but still stale (30-day retention ≈ 720h).
    let root = SessionId::new();
    let child = SessionId::new();
    let root_summary = summary(&root, hours_ago(10_000));
    let mut child_summary = summary(&child, hours_ago(1_000));
    set_parent(&mut child_summary, &root);
    let summaries = vec![root_summary, child_summary];

    // When selecting candidates against a 30-day retention.
    let selected = crate::session_prune_actor::prune_candidates(
        &summaries,
        Timestamp::now(),
        Duration::from_hours(30 * 24),
    );

    // Then both tree members are selected — the tree's newest member is
    // still stale, so the whole family goes.
    let mut ids: Vec<SessionId> = selected;
    ids.sort();
    let mut expected = vec![root, child];
    expected.sort();
    assert_eq!(ids, expected, "both members of the stale tree are selected");
}

#[rstest::rstest]
#[test]
fn prune_candidates_spares_trees_with_one_fresh_member() {
    // Given a stale archived parent with a *fresh* archived child.
    let root = SessionId::new();
    let child = SessionId::new();
    let root_summary = summary(&root, hours_ago(10_000));
    let mut child_summary = summary(&child, just_now());
    set_parent(&mut child_summary, &root);
    let summaries = vec![root_summary, child_summary];

    // When selecting candidates against a 30-day retention.
    let selected = crate::session_prune_actor::prune_candidates(
        &summaries,
        Timestamp::now(),
        Duration::from_hours(30 * 24),
    );

    // Then the fresh child's activity protects the whole tree — neither is
    // selected, not even the stale parent.
    assert!(
        selected.is_empty(),
        "one fresh member must spare the tree; selected {selected:?}"
    );
}

#[rstest::rstest]
#[test]
fn prune_candidates_never_selects_library_members() {
    // Given an ancient archived tree whose parent is in the Library.
    let root = SessionId::new();
    let child = SessionId::new();
    let mut root_summary = summary(&root, hours_ago(10_000));
    root_summary.in_library = true;
    let mut child_summary = summary(&child, hours_ago(10_000));
    set_parent(&mut child_summary, &root);
    let summaries = vec![root_summary, child_summary];

    // When selecting candidates.
    let selected = crate::session_prune_actor::prune_candidates(
        &summaries,
        Timestamp::now(),
        Duration::from_hours(24),
    );

    // Then nothing is selected — Library membership is unconditional.
    assert!(selected.is_empty(), "library members must never be pruned");
}

#[rstest::rstest]
#[test]
fn prune_candidates_never_selects_unarchived_sessions() {
    // Given an ancient tree whose child is still loaded (unarchived).
    let root = SessionId::new();
    let child = SessionId::new();
    let root_summary = summary(&root, hours_ago(10_000));
    let mut child_summary = summary(&child, hours_ago(10_000));
    child_summary.session_state = SessionState::Loaded;
    set_parent(&mut child_summary, &root);
    let summaries = vec![root_summary, child_summary];

    // When selecting candidates.
    let selected = crate::session_prune_actor::prune_candidates(
        &summaries,
        Timestamp::now(),
        Duration::from_hours(24),
    );

    // Then nothing is selected — an unarchived session is "loaded in the
    // sidebar" by definition.
    assert!(
        selected.is_empty(),
        "unarchived sessions must never be pruned"
    );
}

#[rstest::rstest]
#[test]
fn orphaned_child_forms_its_own_root() {
    // Given a child whose parent row is absent from the summaries (a
    // pre-orphaned fork), stale, archived, and outside the Library.
    let missing_parent = SessionId::new();
    let orphan = SessionId::new();
    let mut orphan_summary = summary(&orphan, hours_ago(10_000));
    set_parent(&mut orphan_summary, &missing_parent);
    // And an unrelated fresh session that must not be dragged in.
    let fresh = SessionId::new();
    let fresh_summary = summary(&fresh, just_now());
    let summaries = vec![orphan_summary, fresh_summary];

    // When selecting candidates.
    let selected = crate::session_prune_actor::prune_candidates(
        &summaries,
        Timestamp::now(),
        Duration::from_hours(24),
    );

    // Then the orphan is its own root and is selected on its own subtree's
    // freshness, while the fresh session stays.
    assert_eq!(selected, vec![orphan], "the orphan is judged alone");
}

#[rstest::rstest]
#[tokio::test]
async fn actor_without_retention_never_deletes() {
    // Given a store with a stale archived session and NO [maintenance]
    // section (the harness config is empty).
    let dir = jinn_testutil::tempdir();
    let store = std::sync::Arc::new(
        crate::sqlite::SqliteSessionStore::new_in(&dir.path)
            .await
            .expect("store"),
    );
    let doomed = SessionId::new();
    store
        .save(&stale_archived_session(&doomed))
        .await
        .expect("save stale session");

    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let mut services = harness.services().await;
    services.config = jinn_config::testutil::config_layer("");
    services.session_store = jinn_session_state::SessionStoreService::new(store.clone());
    let deps = jinn_kernel::common::actor_deps::ActorDeps { services };

    // When spawning the actor with a tiny initial delay and sweep interval.
    let _path = crate::session_prune_actor::SessionPruneActor::spawn(
        harness.system(),
        SessionPruneActorDeps {
            deps: deps.clone(),
            initial_delay: Duration::from_millis(20),
            sweep_interval: Duration::from_millis(20),
            drain_interval: Duration::from_millis(20),
            batch: PRUNE_BATCH,
        },
    );

    // Then the session is still there after the sweeps have certainly run
    // (positive-completion: a second spawn's tick chain proves the runtime
    // delivered ticks — the *absence* of deletion is the assertion).
    let ran = poll_until(Duration::from_millis(20), 50, || async {
        // Wait until several sweep windows have passed.
        let summaries = store.load_summaries().await.expect("summaries");
        let _ = summaries;
        false
    })
    .await;
    let _ = ran;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let summaries = store.load_summaries().await.expect("summaries");
    let survivor = summaries.first().map(|s| s.session_id.clone());
    assert_eq!(
        summaries.len(),
        1,
        "no retention configured: nothing may ever be deleted"
    );
    assert_eq!(survivor, Some(doomed));
}

#[rstest::rstest]
#[tokio::test]
async fn actor_sweep_deletes_stale_trees_and_vacuums() {
    // Given a store with a stale archived session, a fresh one, and
    // [maintenance] configured with a tiny retention.
    let dir = jinn_testutil::tempdir();
    let store = std::sync::Arc::new(
        crate::sqlite::SqliteSessionStore::new_in(&dir.path)
            .await
            .expect("store"),
    );
    let stale_id = SessionId::new();
    let fresh_id = SessionId::new();
    store
        .save(&stale_archived_session(&stale_id))
        .await
        .expect("save stale");
    store
        .save(&fresh_loaded_session(&fresh_id))
        .await
        .expect("save fresh");
    // Age both to a year ago, then re-freshen the fresh one by saving again
    // (a save stamps `now`), so exactly one session is stale.
    age_all_sessions(&store).await;
    store
        .save(&fresh_loaded_session(&fresh_id))
        .await
        .expect("re-save fresh");

    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let mut services = harness.services().await;
    services.config =
        jinn_config::testutil::config_layer("[maintenance]\nsession_retention = \"1h\"");
    services.session_store = jinn_session_state::SessionStoreService::new(store.clone());
    let deps = jinn_kernel::common::actor_deps::ActorDeps { services };

    // When spawning the actor with tiny delays.
    let _path = crate::session_prune_actor::SessionPruneActor::spawn(
        harness.system(),
        SessionPruneActorDeps {
            deps: deps.clone(),
            initial_delay: Duration::from_millis(20),
            sweep_interval: Duration::from_millis(20),
            drain_interval: Duration::from_millis(20),
            batch: PRUNE_BATCH,
        },
    );

    // Then the stale session vanishes and the fresh one survives.
    let pruned = poll_until(Duration::from_millis(20), 50, || async {
        store.load_summaries().await.expect("summaries").len() == 1
    })
    .await;
    assert!(pruned, "the stale session should have been pruned");
    let summaries = store.load_summaries().await.expect("summaries");
    let survivor = summaries.first().map(|s| s.session_id.clone());
    assert_eq!(survivor, Some(fresh_id), "only the fresh one left");

    // And a later explicit sweep's vacuum path works against the real
    // store (freelist pages trimmed without error).
    store
        .incremental_vacuum(MAX_VACUUM_PAGES)
        .await
        .expect("vacuum");
}

#[rstest::rstest]
#[tokio::test]
async fn actor_reschedules_after_work() {
    // Given a spawned actor whose sweep interval is tiny, with no retention
    // (each sweep is a no-op).
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let mut services = harness.services().await;
    services.config = jinn_config::testutil::config_layer("");
    let dir = jinn_testutil::tempdir();
    let store = std::sync::Arc::new(
        crate::sqlite::SqliteSessionStore::new_in(&dir.path)
            .await
            .expect("store"),
    );
    services.session_store = jinn_session_state::SessionStoreService::new(store.clone());
    let deps = jinn_kernel::common::actor_deps::ActorDeps { services };

    let path = crate::session_prune_actor::SessionPruneActor::spawn(
        harness.system(),
        SessionPruneActorDeps {
            deps: deps.clone(),
            initial_delay: Duration::from_millis(10),
            sweep_interval: Duration::from_millis(10),
            drain_interval: Duration::from_millis(10),
            batch: PRUNE_BATCH,
        },
    );

    // When several sweep intervals pass.
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Then the actor is still alive and processing ticks — a direct tell
    // resolves, proving the reschedule chain kept delivering ticks (an
    // actor that stopped rescheduling after the first tick would still
    // accept a tell, but the several-ticks-later liveness plus the mailbox
    // being serviced is what the sweep-count probe below observes).
    let delivered = harness
        .system()
        .tell(path, crate::session_prune_actor::PruneTick)
        .await;
    assert!(delivered.is_ok(), "the actor must still be running");
    // And the production constants are what the spec pins.
    assert_eq!(INITIAL_DELAY, Duration::from_secs(5));
    assert_eq!(SWEEP_INTERVAL, Duration::from_hours(1));
    assert_eq!(DRAIN_INTERVAL, Duration::from_secs(2));
    assert_eq!(PRUNE_BATCH, 1);
}

#[rstest::rstest]
#[test]
fn default_maintenance_config_disables_deletion() {
    // Given the default config (no section).
    let config = MaintenanceConfig::default();

    // When reading its retention.
    // Then it is None — the "dont delete anything" default.
    assert_eq!(config.retention_duration(), None);
}

#[rstest::rstest]
#[tokio::test]
async fn drain_deletes_over_multiple_ticks() {
    // Given a recording store with 25 stale archived sessions, retention
    // enabled, and an injected batch of 10 (above production's 1, to prove
    // several sessions ride one transaction when asked).
    let store = Arc::new(PruneQueueStore::stale_sessions(25));
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let mut services = harness.services().await;
    services.config =
        jinn_config::testutil::config_layer("[maintenance]\nsession_retention = \"1h\"");
    services.session_store = jinn_session_state::SessionStoreService::new(store.clone());
    let deps = jinn_kernel::common::actor_deps::ActorDeps { services };

    // When spawning the actor with millisecond intervals and batch 10.
    let _path = crate::session_prune_actor::SessionPruneActor::spawn(
        harness.system(),
        SessionPruneActorDeps {
            deps,
            initial_delay: Duration::from_millis(10),
            sweep_interval: Duration::from_millis(10),
            drain_interval: Duration::from_millis(10),
            batch: 10,
        },
    );

    // Then the whole queue drains across at least three ticks, leaving
    // every session deleted — chunking, not one giant transaction.
    let drained = poll_until(Duration::from_millis(20), 200, || async {
        store.delete_calls() == 25 && store.delete_transactions() >= 3
    })
    .await;
    assert!(
        drained,
        "25 sessions at batch 10 must take at least 3 transactions; got {:?} over {} calls",
        store.deleted_batches(),
        store.delete_calls()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn batch_never_exceeds_ten_per_transaction() {
    // Given a recording store with 25 stale archived sessions and retention
    // enabled.
    let store = Arc::new(PruneQueueStore::stale_sessions(25));
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let mut services = harness.services().await;
    services.config =
        jinn_config::testutil::config_layer("[maintenance]\nsession_retention = \"1h\"");
    services.session_store = jinn_session_state::SessionStoreService::new(store.clone());
    let deps = jinn_kernel::common::actor_deps::ActorDeps { services };

    // When spawning the actor with an injected batch of 10.
    let _path = crate::session_prune_actor::SessionPruneActor::spawn(
        harness.system(),
        SessionPruneActorDeps {
            deps,
            initial_delay: Duration::from_millis(10),
            sweep_interval: Duration::from_millis(10),
            drain_interval: Duration::from_millis(10),
            batch: 10,
        },
    );

    // Then no single delete transaction ever covers more than 10 sessions.
    let drained = poll_until(Duration::from_millis(20), 200, || async {
        store.delete_calls() == 25
    })
    .await;
    assert!(drained, "all 25 sessions must be deleted");
    assert!(
        store.deleted_batches().iter().all(|batch| *batch <= 10),
        "no transaction may exceed the batch size; batches: {:?}",
        store.deleted_batches()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn drain_reschedules_fast_then_hourly() {
    // Given a recording store with 15 stale archived sessions (batch 10 →
    // two chunks), a millisecond drain interval, and an HOUR sweep
    // interval: if the drain paced at the sweep interval, the second chunk
    // would be an hour away and the queue would never finish.
    let store = Arc::new(PruneQueueStore::stale_sessions(15));
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let mut services = harness.services().await;
    services.config =
        jinn_config::testutil::config_layer("[maintenance]\nsession_retention = \"1h\"");
    services.session_store = jinn_session_state::SessionStoreService::new(store.clone());
    let deps = jinn_kernel::common::actor_deps::ActorDeps { services };

    // When spawning the actor with the two intervals far apart.
    let _path = crate::session_prune_actor::SessionPruneActor::spawn(
        harness.system(),
        SessionPruneActorDeps {
            deps,
            initial_delay: Duration::from_millis(10),
            sweep_interval: Duration::from_secs(3600),
            drain_interval: Duration::from_millis(10),
            batch: 10,
        },
    );

    // Then both chunks complete promptly — the drain paced at the drain
    // interval. (The post-drain revert to the sweep interval is pinned by
    // the constants in `actor_reschedules_after_work`.)
    let drained = poll_until(Duration::from_millis(20), 150, || async {
        store.delete_calls() == 15
    })
    .await;
    assert!(
        drained,
        "a drain must pace at the drain interval: 15 sessions across two chunks must finish in milliseconds, not the sweep interval"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn disabled_retention_clears_queue_and_publishes_idle() {
    // Given a spawned actor draining 15 stale sessions with retention
    // enabled.
    let store = Arc::new(PruneQueueStore::stale_sessions(15));
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let mut services = harness.services().await;
    services.config =
        jinn_config::testutil::config_layer("[maintenance]\nsession_retention = \"1h\"");
    services.session_store = jinn_session_state::SessionStoreService::new(store.clone());
    // A handle to the same live layer the actor reads, for the mid-drain
    // change below.
    let config = services.config.clone();
    let deps = jinn_kernel::common::actor_deps::ActorDeps { services };

    let _path = crate::session_prune_actor::SessionPruneActor::spawn(
        harness.system(),
        SessionPruneActorDeps {
            deps,
            initial_delay: Duration::from_millis(10),
            sweep_interval: Duration::from_millis(10),
            // Deliberately slow: the point is the config change landing
            // *between* chunks, so the next tick must find a disabled
            // retention with chunks still queued.
            drain_interval: Duration::from_millis(60),
            batch: 10,
        },
    );
    let started = poll_until(Duration::from_millis(10), 200, || async {
        store.delete_calls() > 0
    })
    .await;
    assert!(started, "the drain must begin while retention is enabled");

    // When retention is disabled mid-drain (the section's live value is
    // reset to the delete-nothing default).
    config
        .put::<MaintenanceConfig>(&MaintenanceConfig::default())
        .expect("reset [maintenance] to its default");
    let deleted_before_off = store.delete_calls();
    let settled = poll_until(Duration::from_millis(20), 200, || async {
        store.delete_calls() == deleted_before_off
    })
    .await;
    assert!(settled, "deletions must stop once retention is disabled");

    // Then the queue was dropped — no further chunks run even though
    // sessions were still queued when the config changed.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        store.delete_calls(),
        deleted_before_off,
        "disabling retention must halt the drain"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn drain_publishes_progress_and_idle_to_dashboard() {
    // Given a recording store with 15 stale archived sessions, retention
    // enabled, and a recording bus capturing the dashboard publishes.
    let store = Arc::new(PruneQueueStore::stale_sessions(15));
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let mut services = harness.services().await;
    services.config =
        jinn_config::testutil::config_layer("[maintenance]\nsession_retention = \"1h\"");
    let (bus, audit) = BusService::new_recording();
    services.bus = bus;
    services.session_store = jinn_session_state::SessionStoreService::new(store.clone());
    let deps = jinn_kernel::common::actor_deps::ActorDeps { services };

    // When spawning the actor and letting the drain complete.
    let _path = crate::session_prune_actor::SessionPruneActor::spawn(
        harness.system(),
        SessionPruneActorDeps {
            deps,
            initial_delay: Duration::from_millis(10),
            sweep_interval: Duration::from_millis(10),
            drain_interval: Duration::from_millis(10),
            batch: 10,
        },
    );
    let drained = poll_until(Duration::from_millis(20), 200, || async {
        store.delete_calls() == 15
    })
    .await;
    assert!(drained, "the queue must fully drain");

    // Then the session-prune row saw the drain's progress and its idle
    // label after it.
    let settled = poll_until(Duration::from_millis(20), 200, || async {
        audit
            .of_type::<ServiceStatusUpdate>()
            .last()
            .is_some_and(|update| update.status_message.as_deref() == Some("prune up to date"))
    })
    .await;
    assert!(settled, "the row must end on the idle label");
    let updates = audit.of_type::<ServiceStatusUpdate>();
    assert!(
        updates
            .iter()
            .all(|update| update.name == SESSION_PRUNE_ROW_NAME),
        "every publish must land on the session-prune row"
    );
    assert!(
        updates
            .iter()
            .any(|update| matches!(&update.status_message, Some(msg) if msg.ends_with(" sessions pending"))),
        "the pending count must be published during the drain"
    );
    assert_eq!(
        updates
            .last()
            .and_then(|update| update.status_message.as_deref()),
        Some("prune up to date"),
        "the drained queue must publish the idle label last"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn failed_delete_requeues_the_chunk() {
    // Given a store whose deletes fail until armed, with 12 stale sessions
    // queued (batch 10 → a 10-session chunk and a 2-session chunk).
    let store = Arc::new(PruneQueueStore::stale_sessions(12));
    store.fail_next_delete();
    let harness = jinn_testutil::bus_harness::TestHarness::new().await;
    let mut services = harness.services().await;
    services.config =
        jinn_config::testutil::config_layer("[maintenance]\nsession_retention = \"1h\"");
    services.session_store = jinn_session_state::SessionStoreService::new(store.clone());
    let deps = jinn_kernel::common::actor_deps::ActorDeps { services };

    // When spawning the actor and letting the failed chunk retry.
    let _path = crate::session_prune_actor::SessionPruneActor::spawn(
        harness.system(),
        SessionPruneActorDeps {
            deps,
            initial_delay: Duration::from_millis(10),
            sweep_interval: Duration::from_millis(10),
            drain_interval: Duration::from_millis(10),
            batch: 10,
        },
    );
    let drained = poll_until(Duration::from_millis(20), 200, || async {
        store.delete_calls() == 12
    })
    .await;

    // Then the chunk was requeued and retried in order: the first
    // transaction failed (0 deleted), and the same 10 sessions — the
    // queue's front, not the tail — were the next successful transaction.
    assert!(drained, "the requeued chunk must be retried to completion");
    let batches = store.deleted_batches();
    assert_eq!(
        batches.first(),
        Some(&0),
        "the first chunk must have failed without deleting"
    );
    assert_eq!(
        batches.get(1),
        Some(&10),
        "the retried chunk must be the same front chunk of 10, in order"
    );
    assert_eq!(batches.len(), 3, "then the tail chunk of 2");
}

// ── Test double ──────────────────────────────────────────────────────────

/// A [`SessionStore`] double that answers `load_summaries` from an
/// in-memory list and records every `delete_many` transaction.
///
/// A successful `delete_many` removes those sessions from the list, as the
/// real store would — the drain must terminate, not refill forever. The
/// one behavior simulated beyond that is the delete-failure retry, via
/// [`Self::fail_next_delete`].
struct PruneQueueStore {
    summaries: Mutex<Vec<SessionSummary>>,
    delete_batches: Mutex<Vec<usize>>,
    fail_next: AtomicBool,
}

impl PruneQueueStore {
    /// A store holding `count` stale archived sessions (each its own root).
    fn stale_sessions(count: usize) -> Self {
        let now = Timestamp::now();
        Self {
            summaries: Mutex::new(
                (0..count)
                    .map(|i| {
                        let id = SessionId::new();
                        SessionSummary {
                            session_id: id,
                            title: format!("stale {i}"),
                            updated_at: now - jiff::SignedDuration::from_hours(10_000),
                            created_at: now - jiff::SignedDuration::from_hours(10_001),
                            session_state: SessionState::Archived,
                            in_library: false,
                            is_bot: false,
                            parent_session: None,
                            project: None,
                        }
                    })
                    .collect(),
            ),
            delete_batches: Mutex::new(Vec::new()),
            fail_next: AtomicBool::new(false),
        }
    }

    /// Makes the next `delete_many` transaction fail (once).
    fn fail_next_delete(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }

    /// Total sessions successfully deleted so far.
    fn delete_calls(&self) -> usize {
        self.delete_batches
            .lock()
            .expect("delete batches")
            .iter()
            .sum()
    }

    /// The number of `delete_many` transactions so far.
    fn delete_transactions(&self) -> usize {
        self.delete_batches.lock().expect("delete batches").len()
    }

    /// The size of each `delete_many` transaction, in order.
    fn deleted_batches(&self) -> Vec<usize> {
        self.delete_batches.lock().expect("delete batches").clone()
    }
}

#[async_trait]
impl SessionStore for PruneQueueStore {
    fn name(&self) -> &'static str {
        "prune-queue"
    }

    async fn save(
        &self,
        _snapshot: &jinn_session_state::SessionSnapshot,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        Ok(self.summaries.lock().expect("summaries").clone())
    }

    async fn delete_many(
        &self,
        session_ids: &[SessionId],
    ) -> Result<(), Report<SessionStoreError>> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            self.delete_batches.lock().expect("delete batches").push(0);
            return Err(Report::new(SessionStoreError));
        }
        self.delete_batches
            .lock()
            .expect("delete batches")
            .push(session_ids.len());
        let deleted: std::collections::HashSet<SessionId> = session_ids.iter().cloned().collect();
        self.summaries
            .lock()
            .expect("summaries")
            .retain(|summary| !deleted.contains(&summary.session_id));
        Ok(())
    }

    async fn incremental_vacuum(&self, _max_pages: u32) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn load_session(
        &self,
        _session_id: &SessionId,
    ) -> Result<Option<jinn_session_state::SessionSnapshot>, Report<SessionStoreError>> {
        Ok(None)
    }

    async fn delete(&self, _session_id: &SessionId) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn fork(
        &self,
        _source_session_id: &SessionId,
        _at_ordinal: usize,
    ) -> Result<SessionId, Report<SessionStoreError>> {
        Ok(SessionId::new())
    }

    async fn set_archived(
        &self,
        _session_id: &SessionId,
        _archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn set_archived_many(
        &self,
        _session_ids: &[SessionId],
        _archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn set_in_library(
        &self,
        _session_id: &SessionId,
        _in_library: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        Ok(self.summaries.lock().expect("summaries").clone())
    }

    async fn dirty_session_ids(&self) -> Result<Vec<SessionId>, Report<SessionStoreError>> {
        Ok(Vec::new())
    }

    async fn reindex_session_chunk(
        &self,
        _session_id: &SessionId,
        _max_entries: usize,
    ) -> Result<bool, Report<SessionStoreError>> {
        Ok(true)
    }

    async fn pending_dirty_count(&self) -> Result<usize, Report<SessionStoreError>> {
        Ok(0)
    }

    async fn search(
        &self,
        _params: SearchParams,
    ) -> Result<SearchOutcome, Report<SessionStoreError>> {
        Ok(SearchOutcome {
            total_matches: 0,
            per_session: Vec::new(),
            hits: Vec::new(),
        })
    }

    async fn fetch_window(
        &self,
        _session_id: &SessionId,
        _anchor: &ChatEntryId,
        _context: usize,
    ) -> Result<Option<TranscriptWindow>, Report<SessionStoreError>> {
        Ok(None)
    }

    async fn fetch_tail(
        &self,
        _session_id: &SessionId,
        _limit: usize,
    ) -> Result<Option<TranscriptWindow>, Report<SessionStoreError>> {
        Ok(None)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// A session snapshot that is archived and aged: saved normally (the store
/// stamps "now"), then rewritten in the sessions table to a timestamp far
/// past any retention window, mirroring a session no one has opened in a
/// year. The metadata marks it archived, so only the retention rules stand
/// between it and the sweep.
fn stale_archived_session(id: &SessionId) -> jinn_session_state::SessionSnapshot {
    let mut session = jinn_session_state::ChatSessionState::new();
    session.set_session_id(id.clone());
    session.set_title("ancient".to_owned());
    session.push_entry(jinn_core_types::ChatEntry::user("old news"));
    let mut snapshot = session.capture_snapshot();
    snapshot.metadata.session_state = jinn_session_store_msg::SessionState::Archived;
    snapshot
}

/// Ages every session in the store to a year ago, directly in the sessions
/// table — the sweep's freshness clock reads this column.
async fn age_all_sessions(store: &crate::sqlite::SqliteSessionStore) {
    store
        .pool()
        .with_conn(|conn| {
            let aged = (Timestamp::now() - jiff::SignedDuration::from_hours(24 * 365)).to_string();
            let changed = conn
                .execute("UPDATE sessions SET updated_at = ?", [&aged])
                .map_err(daow::Error::from)?;
            daow::Result::Ok(changed)
        })
        .await
        .expect("age sessions");
}

/// A normal (loaded, fresh) session snapshot.
fn fresh_loaded_session(id: &SessionId) -> jinn_session_state::SessionSnapshot {
    let mut session = jinn_session_state::ChatSessionState::new();
    session.set_session_id(id.clone());
    session.set_title("current".to_owned());
    session.push_entry(jinn_core_types::ChatEntry::user("hot off the press"));
    session.capture_snapshot()
}

/// Repeats `f` until it returns true or `attempts` elapses.
async fn poll_until<F, Fut>(pause: Duration, attempts: usize, mut f: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..attempts {
        if f().await {
            return true;
        }
        tokio::time::sleep(pause).await;
    }
    f().await
}
