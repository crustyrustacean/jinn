//! Observable behavior tests for the store-owned session actor.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use error_stack::Report;
use jinn_boot_msg::EnvironmentLoaded;
use jinn_core_types::SessionId;
use jinn_domain::common::app_state::AppState;
use jinn_domain::common::bus::test_harness::{TestHarness, await_recorded};
use jinn_domain::common::state::State;
use jinn_domain::feat::session::{SessionStore, SessionStoreError, SessionStoreService};
use jinn_domain::protocol::ChatEntryId;
use jinn_provider_config::ProvidersConfig;
use jinn_session_msg::{SessionArchived, SessionClosed};
use jinn_session_state::{ChatSessionState, SessionSnapshot};
use jinn_session_store_msg::{
    ArchiveSession, PersistSession, SearchOutcome, SearchParams, SessionLoadCompleted,
    SessionLoadRequested, SessionState, SessionSummary, TranscriptWindow,
};

use crate::session_store_actor::{SessionStoreActor, SessionStoreActorDeps};
use crate::sqlite::SqliteSessionStore;

struct ControlledStartupStore {
    summaries: Vec<SessionSummary>,
    snapshots: HashMap<SessionId, SessionSnapshot>,
    session_gates: Mutex<HashMap<SessionId, Arc<tokio::sync::Semaphore>>>,
    tree_summary_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    failed_summaries: AtomicBool,
    failed_session_ids: Mutex<HashSet<SessionId>>,
    requested_session_ids: Mutex<Vec<SessionId>>,
    load_calls: AtomicUsize,
    save_calls: AtomicUsize,
    unarchived_summary_calls: AtomicUsize,
    all_summary_calls: AtomicUsize,
}

impl ControlledStartupStore {
    fn new(entries: &[(SessionId, jiff::Timestamp)]) -> Self {
        let snapshots = entries
            .iter()
            .map(|(session_id, updated_at)| {
                let mut session = ChatSessionState::new();
                session.set_session_id(session_id.clone());
                session.restore_updated_at(*updated_at);
                (session_id.clone(), session.capture_snapshot())
            })
            .collect();
        let summaries = entries
            .iter()
            .map(|(session_id, updated_at)| SessionSummary {
                session_id: session_id.clone(),
                title: session_id.to_string(),
                updated_at: *updated_at,
                created_at: jiff::Timestamp::UNIX_EPOCH,
                session_state: SessionState::Loaded,
                parent_session: None,
                project: None,
            })
            .collect();
        Self {
            summaries,
            snapshots,
            session_gates: Mutex::new(HashMap::new()),
            tree_summary_gate: Mutex::new(None),
            failed_summaries: AtomicBool::new(false),
            failed_session_ids: Mutex::new(HashSet::new()),
            requested_session_ids: Mutex::new(Vec::new()),
            load_calls: AtomicUsize::new(0),
            save_calls: AtomicUsize::new(0),
            unarchived_summary_calls: AtomicUsize::new(0),
            all_summary_calls: AtomicUsize::new(0),
        }
    }

    fn fail_summaries(&self) {
        self.failed_summaries.store(true, Ordering::SeqCst);
    }

    fn fail_session(&self, session_id: SessionId) {
        self.failed_session_ids
            .lock()
            .expect("failed session IDs")
            .insert(session_id);
    }

    fn gate_session_load(&self, session_id: SessionId) {
        self.session_gates
            .lock()
            .expect("session gates")
            .insert(session_id, Arc::new(tokio::sync::Semaphore::new(0)));
    }

    fn release_session_load(&self, session_id: &SessionId) {
        self.session_gates
            .lock()
            .expect("session gates")
            .get(session_id)
            .expect("session load gate")
            .add_permits(1);
    }

    async fn wait_for_session_load(&self, session_id: &SessionId) {
        let observed = poll_until(|| async {
            self.requested_session_ids
                .lock()
                .expect("requested session IDs")
                .contains(session_id)
        })
        .await;
        assert!(observed, "session load should be requested");
    }

    fn gate_tree_summary_load(&self) {
        *self.tree_summary_gate.lock().expect("tree summary gate") =
            Some(Arc::new(tokio::sync::Semaphore::new(0)));
    }

    async fn wait_for_tree_summary_load(&self) {
        let observed =
            poll_until(|| async { self.all_summary_calls.load(Ordering::SeqCst) > 0 }).await;
        assert!(observed, "tree summary load should be requested");
    }
}

#[async_trait]
impl SessionStore for ControlledStartupStore {
    fn name(&self) -> &'static str {
        "controlled-startup"
    }

    async fn save(&self, _snapshot: &SessionSnapshot) -> Result<(), Report<SessionStoreError>> {
        self.save_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        self.all_summary_calls.fetch_add(1, Ordering::SeqCst);
        let gate = self
            .tree_summary_gate
            .lock()
            .expect("tree summary gate")
            .clone();
        if let Some(gate) = gate {
            gate.acquire()
                .await
                .expect("tree summary load gate open")
                .forget();
        }
        if self.failed_summaries.load(Ordering::SeqCst) {
            return Err(Report::new(SessionStoreError));
        }
        Ok(self.summaries.clone())
    }

    async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionSnapshot>, Report<SessionStoreError>> {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        self.requested_session_ids
            .lock()
            .expect("requested session IDs")
            .push(session_id.clone());
        let gate = self
            .session_gates
            .lock()
            .expect("session gates")
            .get(session_id)
            .cloned();
        if let Some(gate) = gate {
            gate.acquire()
                .await
                .expect("session load gate open")
                .forget();
        }
        if self
            .failed_session_ids
            .lock()
            .expect("failed session IDs")
            .contains(session_id)
        {
            return Err(Report::new(SessionStoreError));
        }
        Ok(self.snapshots.get(session_id).cloned())
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

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        self.unarchived_summary_calls.fetch_add(1, Ordering::SeqCst);
        if self.failed_summaries.load(Ordering::SeqCst) {
            return Err(Report::new(SessionStoreError));
        }
        Ok(self.summaries.clone())
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

struct ActorFixture {
    _dir: tempfile::TempDir,
    harness: TestHarness,
    state: State,
    store: Arc<SqliteSessionStore>,
}

async fn actor_fixture() -> ActorFixture {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let store = Arc::new(
        SqliteSessionStore::new_in(dir.path())
            .await
            .expect("session store"),
    );
    let harness = TestHarness::new().await;
    let mut services = harness.services().await;
    services.session_store = SessionStoreService::new(store.clone());
    let state = State::new(AppState::default());
    let _actor = SessionStoreActor::spawn(
        harness.system(),
        SessionStoreActorDeps {
            services,
            state: state.clone(),
        },
    );
    ActorFixture {
        _dir: dir,
        harness,
        state,
        store,
    }
}

async fn poll_until<F, Fut>(mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..80 {
        if condition().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    condition().await
}

async fn controlled_actor_fixture(store: Arc<ControlledStartupStore>) -> (TestHarness, State) {
    let harness = TestHarness::new().await;
    let mut services = harness.services().await;
    services.session_store = SessionStoreService::new(store);
    let state = State::new(AppState::default());
    let _actor = SessionStoreActor::spawn(
        harness.system(),
        SessionStoreActorDeps {
            services,
            state: state.clone(),
        },
    );
    (harness, state)
}

fn empty_providers_config() -> ProvidersConfig {
    ProvidersConfig {
        providers: BTreeMap::new(),
        aliases: Vec::new(),
        default_provider: None,
    }
}

#[rstest::rstest]
#[tokio::test]
async fn startup_hydration_is_visible_before_first_snapshot_completes() {
    // Given a persisted session and a gated summary read.
    let session_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[(
        session_id.clone(),
        jiff::Timestamp::from_second(1).expect("valid timestamp"),
    )]));
    store.gate_session_load(session_id.clone());
    let (harness, state) = controlled_actor_fixture(store.clone()).await;

    // When startup hydration begins.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    store.wait_for_session_load(&session_id).await;

    // Then the shared projection reports hydration active.
    assert!(state.read().session.is_startup_hydrating());
}

#[rstest::rstest]
#[tokio::test]
async fn first_startup_session_is_visible_before_second_snapshot_load() {
    // Given two persisted sessions with the newer session loaded first.
    let newer_id = SessionId::new();
    let older_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[
        (
            older_id.clone(),
            jiff::Timestamp::from_second(1).expect("valid timestamp"),
        ),
        (
            newer_id.clone(),
            jiff::Timestamp::from_second(2).expect("valid timestamp"),
        ),
    ]));
    store.gate_session_load(newer_id.clone());
    store.gate_session_load(older_id.clone());
    let (harness, state) = controlled_actor_fixture(store.clone()).await;

    // When the newer snapshot is released but the older snapshot remains gated.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    store.wait_for_session_load(&newer_id).await;
    store.release_session_load(&newer_id);
    let newer_visible = poll_until(|| async { state.read().session.contains(&newer_id) }).await;
    assert!(newer_visible);

    // Then only the first session is visible while the second read is pending.
    store.wait_for_session_load(&older_id).await;
    assert!(!state.read().session.contains(&older_id));
}

#[rstest::rstest]
#[tokio::test]
async fn startup_sessions_are_inserted_in_existing_recency_order() {
    // Given two persisted sessions with distinct recency timestamps.
    let older_id = SessionId::new();
    let newer_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[
        (
            older_id.clone(),
            jiff::Timestamp::from_second(1).expect("valid timestamp"),
        ),
        (
            newer_id.clone(),
            jiff::Timestamp::from_second(2).expect("valid timestamp"),
        ),
    ]));
    let (harness, _state) = controlled_actor_fixture(store.clone()).await;

    // When startup hydration completes.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    poll_until(|| async { store.load_calls.load(Ordering::SeqCst) == 2 }).await;

    // Then the store reads the snapshots in newest-first order.
    assert_eq!(
        store
            .requested_session_ids
            .lock()
            .expect("requested session IDs")
            .as_slice(),
        &[newer_id, older_id]
    );
}

#[rstest::rstest]
#[tokio::test]
async fn startup_publishes_one_completion_event_per_loaded_session() {
    // Given two persisted sessions.
    let first_id = SessionId::new();
    let second_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[
        (
            first_id.clone(),
            jiff::Timestamp::from_second(1).expect("valid timestamp"),
        ),
        (
            second_id.clone(),
            jiff::Timestamp::from_second(2).expect("valid timestamp"),
        ),
    ]));
    let (harness, _state) = controlled_actor_fixture(store.clone()).await;
    let completed = harness.spawn_recorder::<SessionLoadCompleted>().await;

    // When startup hydration completes.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    let completed = await_recorded(&completed, 2, Duration::from_secs(1)).await;

    // Then one completion event is published for each session.
    assert_eq!(completed.len(), 2);
    assert_eq!(completed[0].session_id, second_id);
    assert_eq!(completed[1].session_id, first_id);
}

#[rstest::rstest]
#[tokio::test]
async fn startup_hydration_clears_before_archived_tree_hydration() {
    // Given a persisted session and a gated archived-tree summary read.
    let session_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[(
        session_id.clone(),
        jiff::Timestamp::from_second(1).expect("valid timestamp"),
    )]));
    store.gate_tree_summary_load();
    let (harness, state) = controlled_actor_fixture(store.clone()).await;

    // When the unarchived session is inserted and tree hydration begins.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    poll_until(|| async { state.read().session.contains(&session_id) }).await;
    store.wait_for_tree_summary_load().await;

    // Then the shared hydration projection is already inactive.
    assert!(!state.read().session.is_startup_hydrating());
}

#[rstest::rstest]
#[tokio::test]
async fn empty_startup_clears_hydration() {
    // Given a store with no unarchived sessions.
    let store = Arc::new(ControlledStartupStore::new(&[]));
    let (harness, state) = controlled_actor_fixture(store.clone()).await;

    // When startup hydration completes.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    poll_until(|| async {
        store.unarchived_summary_calls.load(Ordering::SeqCst) > 0
            && !state.read().session.is_startup_hydrating()
    })
    .await;

    // Then hydration is inactive.
    assert!(!state.read().session.is_startup_hydrating());
}

#[rstest::rstest]
#[tokio::test]
async fn summary_query_failure_clears_hydration() {
    // Given a store whose unarchived summary query fails.
    let store = Arc::new(ControlledStartupStore::new(&[]));
    store.fail_summaries();
    let (harness, state) = controlled_actor_fixture(store.clone()).await;

    // When startup hydration is attempted.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    poll_until(|| async {
        store.unarchived_summary_calls.load(Ordering::SeqCst) > 0
            && !state.read().session.is_startup_hydrating()
    })
    .await;

    // Then hydration is inactive.
    assert!(!state.read().session.is_startup_hydrating());
}

#[rstest::rstest]
#[tokio::test]
async fn individual_load_failure_does_not_abort_remaining_startup_loads() {
    // Given one failed snapshot and one successful newer snapshot.
    let failed_id = SessionId::new();
    let loaded_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[
        (
            failed_id.clone(),
            jiff::Timestamp::from_second(1).expect("valid timestamp"),
        ),
        (
            loaded_id.clone(),
            jiff::Timestamp::from_second(2).expect("valid timestamp"),
        ),
    ]));
    store.fail_session(failed_id);
    let (harness, state) = controlled_actor_fixture(store).await;

    // When startup hydration continues past the failed snapshot.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    poll_until(|| async { state.read().session.contains(&loaded_id) }).await;

    // Then the successful session is still loaded and hydration is inactive.
    assert!(state.read().session.contains(&loaded_id));
    assert!(!state.read().session.is_startup_hydrating());
}

#[rstest::rstest]
#[tokio::test]
async fn startup_preserves_welcome_session_as_active() {
    // Given a persisted session and a running store actor.
    let persisted_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[(
        persisted_id.clone(),
        jiff::Timestamp::from_second(1).expect("valid timestamp"),
    )]));
    let (harness, state) = controlled_actor_fixture(store).await;
    let welcome_id = state.read().session.active_session_id().clone();

    // When startup hydration completes.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    poll_until(|| async { state.read().session.contains(&persisted_id) }).await;

    // Then the persisted session is visible without changing the active welcome session.
    assert_eq!(state.read().session.active_session_id(), &welcome_id);
}

#[rstest::rstest]
#[tokio::test]
async fn startup_does_not_persist_hydrated_sessions() {
    // Given a persisted session and a controllable store.
    let persisted_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[(
        persisted_id.clone(),
        jiff::Timestamp::from_second(1).expect("valid timestamp"),
    )]));
    let (harness, _state) = controlled_actor_fixture(store.clone()).await;

    // When startup hydration completes.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    poll_until(|| async { store.unarchived_summary_calls.load(Ordering::SeqCst) == 1 }).await;
    poll_until(|| async { store.all_summary_calls.load(Ordering::SeqCst) == 1 }).await;

    // Then the startup path does not write the session back to storage.
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 0);
}

#[rstest::rstest]
#[tokio::test]
async fn load_completed_is_published_after_session_is_fully_initialized() {
    // Given a session persisted in the store and removed from the live map.
    let fixture = actor_fixture().await;
    let completed = fixture
        .harness
        .spawn_recorder::<SessionLoadCompleted>()
        .await;
    let session_id = jinn_core_types::SessionId::new();
    let mut stored = ChatSessionState::new();
    stored.set_session_id(session_id.clone());
    stored.set_model(jinn_core_types::ModelSelection::Single(
        "ollama/llama3".to_owned(),
    ));
    stored.push_entry(jinn_core_types::ChatEntry::user("loaded"));
    fixture
        .store
        .save(&stored.capture_snapshot())
        .await
        .expect("save session");
    {
        let mut state = fixture.state.write();
        state.session.remove(&session_id);
        state.session.begin_load(session_id.clone());
    }

    // When the load request is published.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: session_id.clone(),
        })
        .await;

    // Then the completion ID resolves to a fully initialized live session.
    let completed = await_recorded(&completed, 1, Duration::from_secs(1)).await;
    assert_eq!(completed[0].session_id, session_id);
    let state = fixture.state.read();
    let session = state.session.get(&session_id).expect("loaded session");
    assert_eq!(state.session.active_session_id(), &session_id);
    assert!(session.has_interacted());
    assert!(!state.session.is_loading());
}

#[rstest::rstest]
#[tokio::test]
async fn restored_session_is_marked_loaded() {
    // Given a session persisted to storage in the archived state.
    let fixture = actor_fixture().await;
    let session_id = SessionId::new();
    let mut stored = ChatSessionState::new();
    stored.set_session_id(session_id.clone());
    stored.set_model(jinn_core_types::ModelSelection::Single(
        "ollama/llama3".to_owned(),
    ));
    stored.set_session_state(SessionState::Archived);
    fixture
        .store
        .save(&stored.capture_snapshot())
        .await
        .expect("save session");
    fixture
        .store
        .set_archived(&session_id, true)
        .await
        .expect("archive session");

    // When the session is loaded back.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: session_id.clone(),
        })
        .await;
    let restored = poll_until(|| async {
        fixture
            .state
            .read()
            .session
            .get(&session_id)
            .is_some_and(|session| session.session_state() == SessionState::Loaded)
    })
    .await;

    // Then the in-memory session is no longer archived.
    assert!(
        restored,
        "a loaded session must not stay archived in memory"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn loaded_from_archive_appears_in_the_session_list() {
    // Given a session archived in storage and absent from the live map.
    let fixture = actor_fixture().await;
    let session_id = SessionId::new();
    let mut stored = ChatSessionState::new();
    stored.set_session_id(session_id.clone());
    stored.set_model(jinn_core_types::ModelSelection::Single(
        "ollama/llama3".to_owned(),
    ));
    stored.set_session_state(SessionState::Archived);
    stored.push_entry(jinn_core_types::ChatEntry::user("archived work"));
    fixture
        .store
        .save(&stored.capture_snapshot())
        .await
        .expect("save session");
    fixture
        .store
        .set_archived(&session_id, true)
        .await
        .expect("archive session");
    // Given the archived session is still present in the live map, as it is
    // when a previous startup restore brought it back before archiving.
    {
        let mut state = fixture.state.write();
        let restored = stored.clone();
        state.session.insert(restored);
    }

    // When the session is loaded back.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: session_id.clone(),
        })
        .await;
    // When the session carries the state the sidebar lists.
    let listed = poll_until(|| async {
        fixture.state.read().session.iter().any(|(id, session)| {
            id == &session_id && session.session_state() == SessionState::Loaded
        })
    })
    .await;

    // Then the sidebar's loaded-session filter includes it.
    assert!(
        listed,
        "a session loaded from the archive must be Loaded and listed again"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn persist_session_writes_interacted_session_to_store() {
    // Given a running store actor and an interacted session.
    let fixture = actor_fixture().await;
    let session_id = {
        let mut state = fixture.state.write();
        let session = state.active_session_mut();
        session.mark_interacted();
        session.push_entry(jinn_core_types::ChatEntry::user("persist me"));
        session.session_id().clone()
    };

    // When PersistSession is published.
    fixture
        .harness
        .publish(PersistSession {
            session_id: session_id.clone(),
        })
        .await;

    // Then the full session reaches the store service.
    let saved = poll_until(|| async {
        fixture
            .store
            .load_session(&session_id)
            .await
            .ok()
            .flatten()
            .is_some()
    })
    .await;
    assert!(
        saved,
        "PersistSession should write the session to the store"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn archive_session_removes_session_from_state() {
    // Given a running store actor and an active persisted session.
    let fixture = actor_fixture().await;
    let session_id = {
        let mut state = fixture.state.write();
        state.active_session_mut().mark_interacted();
        state.session.active_session_id().clone()
    };

    // When ArchiveSession is published.
    fixture
        .harness
        .publish(ArchiveSession {
            session_id: session_id.clone(),
        })
        .await;

    // Then the session is removed from shared state.
    let removed =
        poll_until(|| async { !fixture.state.read().session.contains(&session_id) }).await;
    assert!(removed, "ArchiveSession should remove the archived session");
}

#[rstest::rstest]
#[tokio::test]
async fn archive_write_failure_leaves_session_live_and_active() {
    // Given an active session and a store whose archive transaction will fail
    // after the session row and history writes.
    let fixture = actor_fixture().await;
    let archived = fixture.harness.spawn_recorder::<SessionArchived>().await;
    let session_id = {
        let mut state = fixture.state.write();
        state.active_session_mut().mark_interacted();
        state
            .active_session_mut()
            .push_entry(jinn_core_types::ChatEntry::user("keep me"));
        state.session.active_session_id().clone()
    };
    fixture
        .store
        .pool()
        .execute("DROP TABLE token_ledger", vec![])
        .await
        .expect("drop token ledger");

    // When archiving the session.
    fixture
        .harness
        .publish(ArchiveSession {
            session_id: session_id.clone(),
        })
        .await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Then the failed transaction leaves the session live, active, and unclosed.
    let state = fixture.state.read();
    assert!(state.session.contains(&session_id));
    assert_eq!(state.session.active_session_id(), &session_id);
    assert!(archived.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn archive_session_publishes_session_archived_event() {
    // Given a running store actor and a recorder for the archive event.
    let fixture = actor_fixture().await;
    let archived = fixture.harness.spawn_recorder::<SessionArchived>().await;
    let session_id = {
        let mut state = fixture.state.write();
        state.active_session_mut().mark_interacted();
        state.session.active_session_id().clone()
    };

    // When ArchiveSession is published.
    fixture
        .harness
        .publish(ArchiveSession {
            session_id: session_id.clone(),
        })
        .await;

    // Then SessionArchived is published for the archived session.
    let archived = await_recorded(&archived, 1, Duration::from_secs(1)).await;
    assert_eq!(archived[0].session_id, session_id);
}

#[rstest::rstest]
#[tokio::test]
async fn archive_session_publishes_session_closed_event() {
    // Given a running store actor and a recorder for the close event.
    let fixture = actor_fixture().await;
    let closed = fixture.harness.spawn_recorder::<SessionClosed>().await;
    let session_id = {
        let mut state = fixture.state.write();
        state.active_session_mut().mark_interacted();
        state.session.active_session_id().clone()
    };

    // When ArchiveSession is published.
    fixture
        .harness
        .publish(ArchiveSession {
            session_id: session_id.clone(),
        })
        .await;

    // Then SessionClosed is published for the archived session.
    let closed = await_recorded(&closed, 1, Duration::from_secs(1)).await;
    assert_eq!(closed[0].session_id, session_id);
}
