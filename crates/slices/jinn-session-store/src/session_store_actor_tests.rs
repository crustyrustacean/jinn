//! Observable behavior tests for the store-owned session actor.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use jinn_boot_msg::EnvironmentLoaded;
use jinn_chat_log_view_msg::LayoutChatSession;
use jinn_core_types::SessionId;
use jinn_domain::common::app_state::AppState;
use jinn_domain::common::bus::HarnessServices;
use jinn_domain::common::state::State;
use jinn_domain::feat::session::{SessionStore, SessionStoreService};
use jinn_provider_config::ProvidersConfig;
use jinn_session_msg::{SessionArchiveFailed, SessionArchived, SessionClosed};
use jinn_session_state::ChatSessionState;
use jinn_session_store_msg::{
    ArchiveSession, ArchiveSessionTree, LoadSessionPickerEntries, PersistSession,
    SessionLoadCompleted, SessionLoadRequested, SessionState,
};
use jinn_testutil::bus_harness::{Recorder, TestHarness, await_recorded};

use crate::session_store_actor::{SessionStoreActor, SessionStoreActorDeps};
use crate::session_store_tests_support::{ControlledStartupStore, poll_until};
use crate::sqlite::SqliteSessionStore;

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
    // The picker cell production's `activate` mints before the spawn; a
    // fixture that skipped it would fail at construction, not at use.
    let session_picker_cell = services
        .slices
        .register(
            jinn_session_store_msg::session_picker_slot(),
            jinn_session_store_msg::SessionPickerState::default(),
        )
        .expect("session picker slot is free in a fresh harness");
    let state = State::new(AppState::default());
    let _actor = SessionStoreActor::spawn(
        harness.system(),
        SessionStoreActorDeps {
            services,
            state: state.clone(),
            session_picker_cell: session_picker_cell.clone(),
        },
    );
    ActorFixture {
        _dir: dir,
        harness,
        state,
        store,
    }
}

type ControlledFixture = (
    TestHarness,
    State,
    jinn_slices::cell::TypedCell<jinn_session_store_msg::SessionPickerState>,
);

async fn controlled_actor_fixture(store: Arc<ControlledStartupStore>) -> ControlledFixture {
    let harness = TestHarness::new().await;
    let mut services = harness.services().await;
    services.session_store = SessionStoreService::new(store);
    let session_picker_cell = services
        .slices
        .register(
            jinn_session_store_msg::session_picker_slot(),
            jinn_session_store_msg::SessionPickerState::default(),
        )
        .expect("session picker slot is free in a fresh harness");
    let state = State::new(AppState::default());
    let _actor = SessionStoreActor::spawn(
        harness.system(),
        SessionStoreActorDeps {
            services,
            state: state.clone(),
            session_picker_cell: session_picker_cell.clone(),
        },
    );
    (harness, state, session_picker_cell)
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
async fn the_picker_is_served_while_startup_session_loads_are_still_outstanding() {
    // Given two persisted sessions whose reads are both gated shut.
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
    store.gate_session_load(first_id.clone());
    store.gate_session_load(second_id.clone());
    let (harness, state, picker_cell) = controlled_actor_fixture(store.clone()).await;

    // When startup hydration is triggered, and the picker's message is
    // published behind it on the same mailbox.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    store.wait_for_session_load(&first_id).await;
    store.wait_for_session_load(&second_id).await;
    harness.publish(LoadSessionPickerEntries).await;
    let picker_served = poll_until(|| async { !picker_cell.read().tree.items().is_empty() }).await;

    // Then the picker is served even though not one session read has finished.
    // Under the old inline loop this message sat in the mailbox until every
    // history had been read — which is the whole stall this pool removes.
    assert!(
        picker_served,
        "the picker must not wait behind startup hydration's reads"
    );
    // And neither session is visible yet, because both reads are still gated.
    assert!(!state.read().session.contains(&first_id));
    assert!(!state.read().session.contains(&second_id));
}

#[rstest::rstest]
#[tokio::test]
async fn the_hydration_flag_clears_only_after_the_last_completion() {
    // Given two persisted sessions, with the newer one's read gated shut.
    let gated_id = SessionId::new();
    let loaded_id = SessionId::new();
    let store = Arc::new(ControlledStartupStore::new(&[
        (
            loaded_id.clone(),
            jiff::Timestamp::from_second(1).expect("valid timestamp"),
        ),
        (
            gated_id.clone(),
            jiff::Timestamp::from_second(2).expect("valid timestamp"),
        ),
    ]));
    store.gate_session_load(gated_id.clone());
    let (harness, state, _picker_cell) = controlled_actor_fixture(store.clone()).await;

    // When the ungated session's read completes while the other is still out.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    let loaded = poll_until(|| async { state.read().session.contains(&loaded_id) }).await;

    // Then hydration is still active: one completion is not all of them.
    assert!(loaded, "the ungated session should have loaded");
    assert!(
        state.read().session.is_startup_hydrating(),
        "hydration must stay active while a read is still outstanding"
    );

    // And when the last read is released, the flag clears.
    store.release_session_load(&gated_id);
    let cleared = poll_until(|| async { !state.read().session.is_startup_hydrating() }).await;
    assert!(cleared, "the last completion must clear the hydration flag");
}

#[rstest::rstest]
#[tokio::test]
async fn a_frozen_tree_member_is_stored_without_reopening_the_hydration_flag() {
    // Given a persisted session whose read resolves, plus a second session that
    // the store reports as a tree member.
    let root_id = SessionId::new();
    let member_id = SessionId::new();
    let store = Arc::new({
        let mut store = ControlledStartupStore::new(&[
            (
                root_id.clone(),
                jiff::Timestamp::from_second(1).expect("valid timestamp"),
            ),
            (
                member_id.clone(),
                jiff::Timestamp::from_second(2).expect("valid timestamp"),
            ),
        ]);
        store.summaries[0].parent_session = None;
        store.summaries[1].parent_session = Some(root_id.clone());
        store
            .archived_only_ids
            .lock()
            .expect("archived-only IDs")
            .push(member_id.clone());
        store
    });
    let (harness, state, _picker_cell) = controlled_actor_fixture(store).await;

    // When startup hydration runs to completion.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    let member_frozen =
        poll_until(|| async { state.read().session.frozen_nodes().contains_key(&member_id) }).await;

    // Then the tree member is stored as a frozen node, not a live session.
    assert!(member_frozen, "the tree member should be frozen");
    assert!(!state.read().session.contains(&member_id));
    // And the hydration flag stays clear: the frozen wave is counted separately
    // and must not resurrect the indicator the unarchived wave already finished.
    assert!(!state.read().session.is_startup_hydrating());
}

#[rstest::rstest]
#[tokio::test]
async fn every_dispatched_load_produces_exactly_one_completion() {
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
    let (harness, _state, _picker_cell) = controlled_actor_fixture(store.clone()).await;
    let completed = harness.spawn_recorder::<SessionLoadCompleted>().await;

    // When startup hydration completes.
    harness
        .publish(EnvironmentLoaded {
            config: empty_providers_config(),
        })
        .await;
    let completed = await_recorded(&completed, 2, Duration::from_secs(2)).await;

    // Then each session reports exactly one completion — no job dropped, none
    // counted twice. A dropped job would strand the hydration flag forever.
    let mut ids = completed
        .iter()
        .map(|msg| msg.session_id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    let mut expected = vec![first_id, second_id];
    expected.sort();
    assert_eq!(ids, expected);
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
    let (harness, state, _picker_cell) = controlled_actor_fixture(store.clone()).await;

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
    let (harness, state, _picker_cell) = controlled_actor_fixture(store.clone()).await;

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
    let (harness, _state, _picker_cell) = controlled_actor_fixture(store.clone()).await;

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
    let (harness, _state, _picker_cell) = controlled_actor_fixture(store.clone()).await;
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
    let (harness, state, _picker_cell) = controlled_actor_fixture(store.clone()).await;

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
    let (harness, state, _picker_cell) = controlled_actor_fixture(store.clone()).await;

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
    let (harness, state, _picker_cell) = controlled_actor_fixture(store.clone()).await;

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
    let (harness, state, _picker_cell) = controlled_actor_fixture(store).await;

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
    let (harness, state, _picker_cell) = controlled_actor_fixture(store).await;
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
    let (harness, _state, _picker_cell) = controlled_actor_fixture(store.clone()).await;

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
            content_width: Some(60),
        })
        .await;

    // Then the completion ID resolves to a fully initialized live session.
    let completed = await_recorded(&completed, 1, Duration::from_secs(1)).await;
    assert_eq!(completed[0].session_id, session_id);
    let state = fixture.state.read();
    let session = state.session.get(&session_id).expect("loaded session");
    assert_eq!(state.session.active_session_id(), &session_id);
    assert!(session.has_interacted());
}

#[rstest::rstest]
#[tokio::test]
async fn a_loaded_session_holds_the_load_guard_for_the_chat_log_measurement() {
    // Given a session persisted in the store and removed from the live map.
    let fixture = actor_fixture().await;
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
            content_width: Some(60),
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Then the guard is still held, because the chat log has not been
    // measured yet — this fixture has no layout workers, which is exactly the
    // path the supervisor's deadline is the backstop for.
    assert!(
        fixture.state.read().session.is_loading(),
        "the load guard must outlive the disk read so the chat log can measure first"
    );
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
            content_width: Some(60),
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
    // Given the session is archived and out of the live map, which is the
    // state an archived session is actually in. A session still sitting in the
    // map is no longer re-read: the store actor recognises it as in memory and
    // only measures it, which would leave it Archived.
    assert!(
        fixture.state.read().session.get(&session_id).is_none(),
        "the archived session starts out of the map"
    );

    // When the session is loaded back.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: session_id.clone(),
            // No width on offer: the caller is not a view switching sessions,
            // so the store actor derives the one it would render at.
            content_width: None,
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

// ---------------------------------------------------------------------------
// Chat log measurement of an in-memory session
// ---------------------------------------------------------------------------

/// A running store actor with a second hydrated session, ready to measure.
///
/// The session store actor is given the measurement recorder, so the layout
/// jobs it dispatches reach it; the workers themselves are not installed, so
/// the cache stays empty and what the measurement produced is observable.
async fn measure_fixture() -> (ActorFixture, Recorder<LayoutChatSession>, SessionId) {
    let fixture = actor_fixture().await;
    let jobs = fixture.harness.spawn_recorder::<LayoutChatSession>().await;
    let target_id = SessionId::new();
    {
        let mut state = fixture.state.write();
        let mut target = ChatSessionState::new();
        target.set_session_id(target_id.clone());
        target.push_entry(jinn_core_types::ChatEntry::user("from the sidebar"));
        state.session.insert(target);
        // The session on screen has a width the next frame will inherit.
        state.active_session_mut().set_content_width(72);
        state.session.begin_load(target_id.clone());
    }
    (fixture, jobs, target_id)
}

#[rstest::rstest]
#[tokio::test]
async fn measuring_an_in_memory_session_dispatches_a_layout_job() {
    // Given a hydrated session waiting to be measured.
    let (fixture, jobs, target_id) = measure_fixture().await;

    // When the measurement is requested.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: target_id.clone(),
            content_width: Some(72),
        })
        .await;

    // Then the session's history is handed to the layout workers.
    let jobs = await_recorded(&jobs, 1, Duration::from_secs(2)).await;
    assert_eq!(jobs.len(), 1, "one session must produce one layout job");
    assert_eq!(jobs[0].session_id, target_id);
    // And the job carries the entries to measure.
    assert_eq!(jobs[0].entries.len(), 1);
    assert_eq!(jobs[0].entries[0].text(), "from the sidebar");
}

#[rstest::rstest]
#[tokio::test]
async fn a_layout_job_from_a_measurement_measures_at_the_width_on_screen() {
    // Given a hydrated session, and an on-screen session that last rendered
    // at 72 columns.
    let (fixture, jobs, target_id) = measure_fixture().await;

    // When the measurement is requested.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: target_id.clone(),
            content_width: Some(72),
        })
        .await;

    // Then the job is measured at the width the next frame will use.
    //
    // The incoming session has never rendered, so its own width is stale;
    // measuring at that would throw the whole measurement away.
    let jobs = await_recorded(&jobs, 1, Duration::from_secs(2)).await;
    assert_eq!(jobs[0].content_width, 72);
}

#[rstest::rstest]
#[tokio::test]
async fn a_layout_job_from_a_measurement_makes_the_session_active() {
    // Given a hydrated session that is not yet active.
    let (fixture, jobs, target_id) = measure_fixture().await;
    let before = fixture.state.read().session.active_session_id().clone();

    // When the measurement is requested.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: target_id.clone(),
            content_width: Some(72),
        })
        .await;
    await_recorded(&jobs, 1, Duration::from_secs(2)).await;

    // Then the completion actor will find it on screen.
    let state = fixture.state.read();
    assert_ne!(
        before, target_id,
        "the fixture must start on another session"
    );
    assert_eq!(state.session.active_session_id(), &target_id);
}

#[rstest::rstest]
#[tokio::test]
async fn measuring_an_absent_session_clears_the_load_guard() {
    // Given a session id that is not in the live map, with the guard held.
    let fixture = actor_fixture().await;
    let missing_id = SessionId::new();
    {
        let mut state = fixture.state.write();
        state.session.begin_load(missing_id.clone());
    }
    let jobs = fixture.harness.spawn_recorder::<LayoutChatSession>().await;

    // When the measurement is requested for it.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: missing_id.clone(),
            content_width: Some(72),
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Then the guard is cleared, so the user is not stranded on a spinner.
    assert!(
        !fixture.state.read().session.is_loading(),
        "a measurement that can never run must not hold the guard"
    );
    // And no work is dispatched for a session that does not exist.
    assert!(
        jobs.is_empty(),
        "an absent session must not produce a layout job"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn measuring_an_in_memory_session_never_reads_it_from_the_store() {
    // Given a hydrated session that is not persisted.
    let (fixture, jobs, target_id) = measure_fixture().await;

    // When the measurement is requested.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: target_id.clone(),
            content_width: Some(72),
        })
        .await;
    await_recorded(&jobs, 1, Duration::from_secs(2)).await;

    // Then the store was never asked for it — the whole point of measuring an
    // in-memory session rather than routing through a load.
    let stored = fixture.store.load_session(&target_id).await.ok().flatten();
    assert!(
        stored.is_none(),
        "the session was never persisted, so no disk read could have served it"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn a_measured_request_clears_the_load_guard_end_to_end() {
    // Given the full layout subsystem, and an in-memory session to measure.
    let fixture = actor_fixture().await;
    jinn_chat_log_view::kernel_element::install_layout_actors(
        fixture.harness.system(),
        fixture.state.clone(),
    );
    let target_id = SessionId::new();
    {
        let mut state = fixture.state.write();
        let mut target = ChatSessionState::new();
        target.set_session_id(target_id.clone());
        for index in 0..40 {
            target.push_entry(jinn_core_types::ChatEntry::user(format!(
                "a reasonably long message number {index} that will wrap a few times"
            )));
        }
        state.session.insert(target);
        state.active_session_mut().set_content_width(72);
        state.session.begin_load(target_id.clone());
    }

    // When the measurement is requested.
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: target_id.clone(),
            content_width: Some(72),
        })
        .await;

    // Then the guard is released by the real worker pool and completion actor.
    let released = poll_until(|| async { !fixture.state.read().session.is_loading() }).await;
    assert!(
        released,
        "the real layout pipeline must clear the guard it was asked to satisfy"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn measuring_after_the_frontend_switched_measures_at_a_usable_width() {
    // Given the full layout subsystem and an in-memory session.
    let fixture = actor_fixture().await;
    jinn_chat_log_view::kernel_element::install_layout_actors(
        fixture.harness.system(),
        fixture.state.clone(),
    );
    let target_id = SessionId::new();
    {
        let mut state = fixture.state.write();
        let mut target = ChatSessionState::new();
        target.set_session_id(target_id.clone());
        for index in 0..40 {
            target.push_entry(jinn_core_types::ChatEntry::user(format!(
                "a reasonably long message number {index} that will wrap a few times"
            )));
        }
        state.session.insert(target);
        // The session on screen last rendered at 72 columns.
        state.active_session_mut().set_content_width(72);
    }

    // When the frontend switches first, then asks for the measurement — which
    // is the order the sidebar activation uses.
    let jobs = fixture.harness.spawn_recorder::<LayoutChatSession>().await;
    fixture.state.write().session.set_active(target_id.clone());
    fixture
        .harness
        .publish(SessionLoadRequested {
            session_id: target_id.clone(),
            content_width: Some(72),
        })
        .await;

    // Then the job is measured at the width the chat log is rendering at.
    let jobs = await_recorded(&jobs, 1, Duration::from_secs(2)).await;
    assert_eq!(jobs[0].content_width, 72);
}

#[rstest::rstest]
#[tokio::test]
async fn archive_tree_aborted_by_busy_member_reports_failure_for_every_member() {
    // Given a parent session with one busy child, both live in the session map.
    let fixture = actor_fixture().await;
    let mut parent = ChatSessionState::new();
    let parent_session_id = parent.session_id().clone();
    let mut child = ChatSessionState::new();
    let child_id = child.session_id().clone();
    child.set_parent_session(parent_session_id.clone());
    child.begin_busy();
    parent.set_parent_session(SessionId::new());
    {
        let mut state = fixture.state.write();
        state.session.insert(parent);
        state.session.insert(child);
    }
    let failures = fixture
        .harness
        .spawn_recorder::<SessionArchiveFailed>()
        .await;

    // When archiving the tree, whose own guard rejects the busy member.
    fixture
        .harness
        .publish(ArchiveSessionTree {
            root: parent_session_id.clone(),
        })
        .await;
    let reported = await_recorded(&failures, 2, Duration::from_secs(2)).await;

    // Then both members are reported, so both tinted rows are cleared.
    let mut ids = reported
        .iter()
        .map(|msg| msg.session_id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    let mut expected = vec![parent_session_id, child_id];
    expected.sort();
    assert_eq!(ids, expected, "every tinted member must be cleared");
}

/// Activation is one command, and the store actor decides what it means.
///
/// A caller cannot: only the actor that owns the session map knows what is
/// loaded, and a caller that guesses pays for a redundant disk read of a
/// session it already had — which is what the picker's Enter did before the two
/// commands were merged.
mod activation_tests {
    #![allow(
        unused_mut,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    /// A persisted session, out of the live map.
    async fn persisted_but_absent(fixture: &ActorFixture) -> jinn_core_types::SessionId {
        let session_id = jinn_core_types::SessionId::new();
        let mut stored = ChatSessionState::new();
        stored.set_session_id(session_id.clone());
        stored.set_model(jinn_core_types::ModelSelection::Single(
            "ollama/llama3".to_owned(),
        ));
        stored.push_entry(jinn_core_types::ChatEntry::user("work"));
        fixture
            .store
            .save(&stored.capture_snapshot())
            .await
            .expect("save session");
        fixture.state.write().session.remove(&session_id);
        session_id
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activating_an_absent_session_reads_storage() {
        // Given a session persisted but not in memory.
        let fixture = actor_fixture().await;
        let session_id = persisted_but_absent(&fixture).await;

        // When it is activated.
        fixture
            .harness
            .publish(SessionLoadRequested {
                session_id: session_id.clone(),
                content_width: Some(60),
            })
            .await;
        let loaded = poll_until(|| async {
            fixture
                .state
                .read()
                .session
                .get(&session_id)
                .is_some_and(|s| s.session_state() == SessionState::Loaded)
        })
        .await;

        // Then it was read from disk and is now in memory.
        assert!(loaded, "an absent session must be read from storage");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activating_a_session_already_in_memory_skips_storage() {
        // Given a session sitting in the live map, and a store that counts the
        // reads it is asked for.
        let store = Arc::new(ControlledStartupStore::new(&[]));
        let (harness, state, _picker) = controlled_actor_fixture(store.clone()).await;
        let session_id = jinn_core_types::SessionId::new();
        {
            let mut guard = state.write();
            let mut session = ChatSessionState::new();
            session.set_session_id(session_id.clone());
            session.push_entry(jinn_core_types::ChatEntry::user("already here"));
            guard.session.insert(session);
            guard.session.begin_load(session_id.clone());
        }
        let before = store.load_calls.load(std::sync::atomic::Ordering::SeqCst);

        // When it is activated with no width on offer, the way Discord asks.
        harness
            .publish(SessionLoadRequested {
                session_id: session_id.clone(),
                content_width: None,
            })
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Then the store was never asked for it.
        let after = store.load_calls.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            after, before,
            "a session already in memory must not be re-read from storage"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activating_a_session_in_memory_arms_the_measurement() {
        // Given a session in memory with the load guard armed, as a caller does.
        let fixture = actor_fixture().await;
        let session_id = persisted_but_absent(&fixture).await;
        {
            let mut state = fixture.state.write();
            state.session.begin_load(session_id.clone());
        }

        // When it is activated.
        fixture
            .harness
            .publish(SessionLoadRequested {
                session_id: session_id.clone(),
                content_width: Some(60),
            })
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Then the guard is still held: the in-memory path measures, and this
        // fixture has no layout workers, so the measurement never completes.
        // That it got that far is the assertion — the guard was not cleared on
        // arrival, which is what an unmeasured activation looks like.
        assert!(
            fixture.state.read().session.is_loading(),
            "an in-memory activation must measure rather than clear the guard"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activating_a_session_that_vanished_clears_the_guard() {
        // Given a load guard armed for a session that is not in memory, as it
        // would be after an eviction raced the activation.
        let fixture = actor_fixture().await;
        let session_id = jinn_core_types::SessionId::new();
        {
            let mut state = fixture.state.write();
            state.session.begin_load(session_id.clone());
        }

        // When it is activated.
        fixture
            .harness
            .publish(SessionLoadRequested {
                session_id: session_id.clone(),
                content_width: Some(60),
            })
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Then the guard is released rather than left spinning on a measurement
        // that will never run.
        assert!(
            !fixture.state.read().session.is_loading(),
            "an activation that found nothing must release the guard"
        );
    }
}
