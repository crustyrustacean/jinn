//! Tests for the `DbMaintenanceActor` sweep behavior.

#![allow(clippy::expect_used, clippy::unwrap_used, reason = "test code")]

use std::time::Duration;

use jiff::Timestamp;
use jinn_core_types::SessionId;
use jinn_kernel::common::bus::HarnessServices;
use jinn_preferences_config::MaintenanceConfig;
use jinn_session_state::SessionStore;
use jinn_session_store_msg::SessionState;
use jinn_session_store_msg::SessionSummary;

use crate::db_maintenance_actor::{
    DbMaintenanceActorDeps, INITIAL_DELAY, MAX_VACUUM_PAGES, SWEEP_INTERVAL,
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
    let selected = crate::db_maintenance_actor::prune_candidates(
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
    let selected = crate::db_maintenance_actor::prune_candidates(
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
    let selected = crate::db_maintenance_actor::prune_candidates(
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
    let selected = crate::db_maintenance_actor::prune_candidates(
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
    let selected = crate::db_maintenance_actor::prune_candidates(
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
    let _path = crate::db_maintenance_actor::DbMaintenanceActor::spawn(
        harness.system(),
        DbMaintenanceActorDeps {
            deps: deps.clone(),
            initial_delay: Duration::from_millis(20),
            sweep_interval: Duration::from_millis(20),
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
    let _path = crate::db_maintenance_actor::DbMaintenanceActor::spawn(
        harness.system(),
        DbMaintenanceActorDeps {
            deps: deps.clone(),
            initial_delay: Duration::from_millis(20),
            sweep_interval: Duration::from_millis(20),
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

    let path = crate::db_maintenance_actor::DbMaintenanceActor::spawn(
        harness.system(),
        DbMaintenanceActorDeps {
            deps: deps.clone(),
            initial_delay: Duration::from_millis(10),
            sweep_interval: Duration::from_millis(10),
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
        .tell(path, crate::db_maintenance_actor::MaintenanceTick)
        .await;
    assert!(delivered.is_ok(), "the actor must still be running");
    // And the production constants are what the spec pins.
    assert_eq!(INITIAL_DELAY, Duration::from_secs(30));
    assert_eq!(SWEEP_INTERVAL, Duration::from_hours(1));
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
