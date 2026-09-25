#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::path::PathBuf;

use crate::common::actor_deps::ActorDeps;
use crate::common::app_paths::AppPaths;
use crate::common::app_state::AppState;
use crate::common::bus::test_harness::TestHarness;
use crate::common::services::test_services::TestServices;
use crate::common::state::State;
use jinn_chat_input_msg::ListDirectory;
use jinn_core_types::SessionId;

use super::directory_lister_actor::{DirectoryListerActor, DirectoryListerActorDeps};

// ── DirectoryListerActor (spawned harness) ─────────────────────────────────

async fn create_harness() -> (TestHarness, State, ActorDeps) {
    let harness = TestHarness::new().await;
    let state = State::new(AppState::default());
    let mut services = TestServices::builder()
        .paths(AppPaths::new_in(std::path::Path::new("")))
        .build();
    services.bus = harness.bus();
    services.trouper_system = harness.system().clone();
    let deps = ActorDeps { services };
    (harness, state, deps)
}

fn make_temp_dir(entries: &[(&str, bool)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jinn-file-lister-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    for (name, is_dir) in entries {
        let path = dir.join(name);
        if *is_dir {
            std::fs::create_dir_all(&path).expect("create subdir");
        } else {
            std::fs::write(&path, b"x").expect("create file");
        }
    }
    dir
}

async fn wait_for_list_complete(state: &State) {
    // The actor clears `loading` when it finishes (success or error).
    // Poll until that happens, with a timeout.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while state.read().frontend.file_picker.loading {
        assert!(
            std::time::Instant::now() <= deadline,
            "timed out waiting for DirectoryListerActor to finish"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[expect(clippy::unused_async, reason = "async for test-helper symmetry")]
async fn spawn_actor(deps: &ActorDeps, state: &State) -> trouper::actor::ActorPath {
    // The path is a placeholder; the actor self-subscribes to the domain
    // topic at its static path. Tests drive it via bus publishes only.
    DirectoryListerActor::spawn(
        &deps.services.trouper_system,
        DirectoryListerActorDeps {
            deps: deps.clone(),
            state: state.clone(),
        },
    )
}

#[rstest::rstest]
#[tokio::test]
async fn actor_reads_directory_entries_into_file_picker() {
    // Given a temp dir with a file and a subdirectory.
    let dir = make_temp_dir(&[("alpha.txt", false), ("subdir", true)]);
    let (harness, state, deps) = create_harness().await;
    let _actor = spawn_actor(&deps, &state).await;

    // Set the expected request id and mark loading.
    state.with_file_picker(|ops| {
        ops.file_picker().expected_request_id = 1;
        ops.file_picker().loading = true;
    });

    // When the actor lists the directory.
    harness
        .publish(ListDirectory {
            session_id: SessionId::new(),
            path: dir.clone(),
            request_id: 1,
        })
        .await;

    wait_for_list_complete(&state).await;

    // Then the file picker is populated and loading is cleared.
    let entries = state.read().frontend.file_picker.entries.clone();
    let loading = state.read().frontend.file_picker.loading;
    assert!(
        !loading,
        "loading should be cleared after a successful read"
    );
    assert!(
        entries.iter().any(|e| e.name == "alpha.txt" && !e.is_dir),
        "file entry should be present: {entries:?}"
    );
    assert!(
        entries.iter().any(|e| e.name == "subdir" && e.is_dir),
        "dir entry should be present: {entries:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn actor_drops_stale_reply_when_request_id_mismatches() {
    // Given a temp dir with one file.
    let dir = make_temp_dir(&[("stale.txt", false)]);
    let (harness, state, deps) = create_harness().await;
    let _actor = spawn_actor(&deps, &state).await;

    // The expected id is 5, but we send a request with id 1 (stale).
    state.with_file_picker(|ops| {
        ops.file_picker().expected_request_id = 5;
        ops.file_picker().loading = true;
    });

    // When the actor processes a request whose id does not match.
    harness
        .publish(ListDirectory {
            session_id: SessionId::new(),
            path: dir,
            request_id: 1, // stale
        })
        .await;

    // The actor processes the stale request but does NOT clear loading.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Then the stale reply is dropped: entries stay empty, loading unchanged.
    let entries = state.read().frontend.file_picker.entries.clone();
    let loading = state.read().frontend.file_picker.loading;
    assert!(entries.is_empty(), "stale reply must not populate entries");
    assert!(loading, "stale reply must not clear loading");
}

#[rstest::rstest]
#[tokio::test]
async fn actor_returns_empty_for_nonexistent_directory() {
    // Given an actor and a path that does not exist.
    let (harness, state, deps) = create_harness().await;
    let _actor = spawn_actor(&deps, &state).await;
    state.with_file_picker(|ops| {
        ops.file_picker().expected_request_id = 1;
        ops.file_picker().loading = true;
    });
    let bogus = PathBuf::from("/this/path/does/not/exist/jinn-test");

    // When the actor lists the nonexistent directory.
    harness
        .publish(ListDirectory {
            session_id: SessionId::new(),
            path: bogus,
            request_id: 1,
        })
        .await;

    wait_for_list_complete(&state).await;

    // Then the entries are empty (not an error), loading cleared.
    let entries = state.read().frontend.file_picker.entries.clone();
    let loading = state.read().frontend.file_picker.loading;
    assert!(entries.is_empty(), "nonexistent dir yields empty entries");
    assert!(!loading, "loading should be cleared even on read error");
}

#[rstest::rstest]
#[tokio::test]
async fn actor_lists_hidden_files() {
    // Given a temp dir with a dotfile and a regular file.
    let dir = make_temp_dir(&[(".hidden", false), ("visible.txt", false)]);
    let (harness, state, deps) = create_harness().await;
    let _actor = spawn_actor(&deps, &state).await;
    state.with_file_picker(|ops| {
        ops.file_picker().expected_request_id = 1;
        ops.file_picker().loading = true;
    });

    // When the actor lists the directory.
    harness
        .publish(ListDirectory {
            session_id: SessionId::new(),
            path: dir,
            request_id: 1,
        })
        .await;

    wait_for_list_complete(&state).await;

    // Then the dotfile appears in the listing (hidden files shown).
    let entries = state.read().frontend.file_picker.entries.clone();
    assert!(
        entries.iter().any(|e| e.name == ".hidden"),
        "hidden files should be listed: {entries:?}"
    );
}
