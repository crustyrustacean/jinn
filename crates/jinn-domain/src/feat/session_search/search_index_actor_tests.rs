//! Tests for the `SearchIndexActor` tick/drain behavior.

#![allow(clippy::expect_used, clippy::unwrap_used, reason = "test code")]

use std::time::Duration;

use crate::common::bus::test_harness::TestHarness;
use crate::common::root_supervisor::RootSupervisor;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::session_store::SessionStoreService;
use crate::feat::session::session_store::SqliteSessionStore;
use crate::feat::session_search::search_index_actor::{REINDEX_INTERVAL, SearchIndexActorDeps};
use crate::protocol::SessionId;

/// Builds actor deps whose session store is a real SQLite store in a temp
/// dir, so drains exercise the actual dirty-marker → FTS pipeline. Returns
/// the concrete store too, for tests that need direct DB access.
async fn sqlite_actor_deps() -> (
    tempfile::TempDir,
    crate::common::actor_deps::ActorDeps,
    std::sync::Arc<SqliteSessionStore>,
) {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let store = std::sync::Arc::new(SqliteSessionStore::new_in(dir.path()).await.expect("store"));
    let harness = TestHarness::new().await;
    let mut services = harness.services().await;
    services.session_store = SessionStoreService::new(store.clone());
    let deps = crate::common::actor_deps::ActorDeps { services };
    (dir, deps, store)
}

/// A two-entry session whose entries mention "needle" so it is findable.
fn needle_session(id: &SessionId) -> ChatSessionState {
    let mut session = ChatSessionState::new();
    session.set_session_id(id.clone());
    session.set_title("tick".to_owned());
    session.push_entry(crate::protocol::ChatEntry::user(
        "the needle thread pulls through",
    ));
    session.push_entry(crate::protocol::ChatEntry::assistant(
        "stitching the needle into place",
    ));
    session
}

async fn search_all(store: &SessionStoreService) -> crate::feat::session_search::SearchOutcome {
    store
        .search(crate::feat::session_search::SearchParams {
            query: "needle".to_owned(),
            session_ids: Vec::new(),
            roles: Vec::new(),
            since: None,
            until: None,
            limit: 10,
        })
        .await
        .expect("search")
}

/// Expects `f` to succeed before `attempts` polls of `pause` elapse.
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

#[rstest::rstest]
#[tokio::test]
async fn startup_drain_indexes_all_dirty_sessions() {
    // Given a dirty session in the store (fresh saves seed the dirty table).
    let (_dir, deps, store) = sqlite_actor_deps().await;
    let session_id = SessionId::new();
    deps.services
        .session_store
        .save(&needle_session(&session_id))
        .await
        .expect("save");

    // When spawning the actor (its on_start kicks an immediate drain).
    let root = RootSupervisor::spawn_root().await;
    let _actor = crate::feat::session_search::search_index_actor::spawn_search_index_actor(
        SearchIndexActorDeps {
            deps: deps.clone(),
            interval: REINDEX_INTERVAL,
        },
        &root,
    )
    .await;

    // Then the startup drain indexes the session without waiting a tick.
    let indexed = poll_until(Duration::from_millis(50), 40, || async {
        search_all(&deps.services.session_store).await.total_matches == 2
    })
    .await;
    assert!(indexed, "startup drain should index the dirty session");
}

#[rstest::rstest]
#[tokio::test]
async fn tick_loop_picks_up_sessions_marked_after_startup() {
    // Given a running actor with a tiny injected interval.
    let (_dir, deps, store) = sqlite_actor_deps().await;
    let root = RootSupervisor::spawn_root().await;
    let _actor = crate::feat::session_search::search_index_actor::spawn_search_index_actor(
        SearchIndexActorDeps {
            deps: deps.clone(),
            interval: Duration::from_millis(50),
        },
        &root,
    )
    .await;

    // When a session is saved after the actor is already running.
    let session_id = SessionId::new();
    deps.services
        .session_store
        .save(&needle_session(&session_id))
        .await
        .expect("save");

    // Then a subsequent tick indexes it (no manual drain involved).
    let indexed = poll_until(Duration::from_millis(50), 40, || async {
        search_all(&deps.services.session_store).await.total_matches == 2
    })
    .await;
    assert!(indexed, "tick loop should converge on newly dirty sessions");
}

#[rstest::rstest]
#[tokio::test]
async fn failed_drain_leaves_marker_and_next_drain_recovers() {
    // Given a session whose marker is re-inserted directly (simulating
    // pending work left behind by a failed drain).
    let (_dir, deps, store) = sqlite_actor_deps().await;
    let session_id = SessionId::new();
    deps.services
        .session_store
        .save(&needle_session(&session_id))
        .await
        .expect("save");

    // Drain once so the index is built, then re-mark the session dirty the
    // way a crashed drain would leave it (insert straight into fts_dirty).
    deps.services
        .session_store
        .reindex_dirty_sessions()
        .await
        .expect("priming drain");
    store
        .pool()
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO fts_dirty(session_id) VALUES (?) \
                 ON CONFLICT(session_id) DO NOTHING",
                rusqlite::params![session_id.to_string()],
            )
            .map_err(daow::Error::from)
        })
        .await
        .expect("mark dirty");

    // When the actor runs with a tiny interval.
    let root = RootSupervisor::spawn_root().await;
    let _actor = crate::feat::session_search::search_index_actor::spawn_search_index_actor(
        SearchIndexActorDeps {
            deps: deps.clone(),
            interval: Duration::from_millis(50),
        },
        &root,
    )
    .await;

    // Then a tick drains the pending work to zero and the session stays
    // searchable throughout.
    let drained = poll_until(Duration::from_millis(50), 40, || async {
        deps.services
            .session_store
            .reindex_dirty_sessions()
            .await
            .expect("drain")
            == 0
    })
    .await;
    assert!(drained, "actor should drain the re-marked session");
    assert_eq!(
        search_all(&deps.services.session_store).await.total_matches,
        2,
        "session remains searchable"
    );
}

#[rstest::rstest]
#[test]
fn production_interval_is_five_seconds() {
    // Given the production interval constant.
    // Then it is exactly 5 seconds (matches the record/plan contract).
    assert_eq!(REINDEX_INTERVAL, Duration::from_secs(5));
}
