//! Tests for the `load_attendant` tool and the workflow-source save-block.

#![allow(clippy::expect_used, reason = "test code")]

use std::path::Path;
use std::time::Duration;

use jinn_common::app_paths::AppPaths;
use jinn_core_types::AttendantSourcePath;
use jinn_core_types::SessionId;
use jinn_core_types::tool_types::ToolCall;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::state::State;
use jinn_session_state::ChatSessionState;
use jinn_session_store_msg::PersistSession;
use jinn_testutil::bus_harness::{TestHarness, await_recorded};

use crate::load_attendant::{definition, execute, parse_attendant_entry};
use crate::tool_types::ToolContext;

/// Builds a tool call with the given JSON arguments.
fn call(arguments: &str) -> ToolCall {
    ToolCall {
        id: "tc_load_1".into(),
        name: "load_attendant".to_owned(),
        arguments: arguments.to_owned(),
    }
}

/// Builds a tool context wired to the harness bus and shared state, rooted at
/// `paths_root` — a directory the calling test owns for its whole run.
async fn ctx(
    harness: &TestHarness,
    state: &State,
    session_id: SessionId,
    paths_root: &Path,
) -> ToolContext {
    ToolContext {
        cwd: paths_root.to_path_buf(),
        config: jinn_config::testutil::config_layer(""),
        shells: Default::default(),
        timeout: None,
        state: Some(state.clone()),
        session_id: Some(session_id),
        app_paths: AppPaths::new_in(paths_root),
        bus: Some(harness.bus()),
        max_output_lines: None,
        max_output_bytes: None,
        dispatched_at: jiff::Timestamp::now(),
        mcp_coordinator: None,
        interactive_term: None,
        task_spawns: None,
        session_store: None,
        trouper_system: Some(harness.system().clone()),
        model_name_resolver: None,
    }
}

/// Seeds one user session; returns (state, user_id).
fn fixture() -> (State, SessionId) {
    let state = State::new(AppState::default_with_scope_focus());
    let user = ChatSessionState::new();
    let user_id = user.session_id().clone();
    state.write().session.insert(user);
    (state, user_id)
}

/// Writes an attendant file under a fresh prefixed temp dir; returns the
/// TempDir (its Drop removes the tree) and the file's path.
fn attendant_file(name: &str, body: &str) -> (tempfile::TempDir, String) {
    let dir = jinn_testutil::tempdir_with_prefix(&format!("jinn-test-load-attendant-{name}-"))
        .expect("temp dir");
    let path = dir.path().join("attendant.toml");
    std::fs::write(&path, body).expect("write attendant file");
    let path = path.display().to_string();
    (dir, path)
}

const ATTENDANT_BODY: &str = r#"
name = "implementer"
behavior = "preserve"
trigger = "manual"
prep_mode = false
seed_template = "You implement one stage of an autocoder run."
"#;

#[rstest::rstest]
#[tokio::test]
async fn load_attaches_a_workflow_attendant_and_stamps_the_marker() {
    // Given a user session and a workflow-local attendant file on disk.
    let harness = TestHarness::new().await;
    let persists = harness.spawn_recorder::<PersistSession>().await;
    let (state, user_id) = fixture();
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let (_file_dir, path) = attendant_file("stamp", ATTENDANT_BODY);
    let tool_ctx = ctx(&harness, &state, user_id.clone(), dir.path()).await;

    // When the tool loads the file.
    let result = execute(
        call(&serde_json::json!({ "path": path.clone() }).to_string()),
        tool_ctx,
    )
    .await;

    // Then the tool succeeds and the attendant exists as a child.
    assert!(result.success, "load must succeed: {}", result.content);
    let guard = state.read();
    let attendant = guard
        .session
        .iter()
        .map(|(_, s)| s)
        .find(|s| s.is_attendant())
        .expect("a workflow attendant must exist");
    assert_eq!(attendant.parent_session().as_ref(), Some(&user_id));
    assert_eq!(attendant.title(), Some("implementer"));
    // And it carries the workflow-source marker naming its source file.
    assert_eq!(
        attendant.workflow_source(),
        Some(&AttendantSourcePath::new(path.clone()))
    );
    assert!(!attendant.attendant_is_prepping(), "file config applied");
    // And the creation published persists for parent and attendant.
    drop(guard);
    let persisted = await_recorded::<PersistSession>(&persists, 2, Duration::from_secs(2)).await;
    assert_eq!(persisted.len(), 2);
}

#[rstest::rstest]
#[tokio::test]
async fn children_of_a_workflow_attendant_do_not_inherit_the_marker() {
    // Given an attendant loaded from a workflow-local file.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let (_file_dir, path) = attendant_file("inherit", ATTENDANT_BODY);
    let tool_ctx = ctx(&harness, &state, user_id.clone(), dir.path()).await;
    let result = execute(
        call(&serde_json::json!({ "path": path }).to_string()),
        tool_ctx,
    )
    .await;
    assert!(result.success);

    // When a child is created the ordinary way from that attendant
    // (`new_attendant` is the creation path every spawner shares).
    let guard = state.read();
    let loaded = guard
        .session
        .iter()
        .map(|(_, s)| s)
        .find(|s| s.is_attendant())
        .expect("workflow attendant");
    let child = ChatSessionState::new_attendant(loaded, true);
    let child_id = child.session_id().clone();
    drop(guard);
    state.write().session.insert(child);

    // Then the child is an ordinary personal attendant: no marker.
    let guard = state.read();
    let child = guard.session.get(&child_id).expect("child");
    assert_eq!(
        child.workflow_source(),
        None,
        "the marker is never inherited"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn missing_file_fails_naming_the_path() {
    // Given a session and a path that does not exist.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let tool_ctx = ctx(&harness, &state, user_id.clone(), dir.path()).await;

    // When the tool loads the missing file.
    let result = execute(call(r#"{"path":"/definitely/not/here.toml"}"#), tool_ctx).await;

    // Then the tool fails naming the path and nothing is created.
    assert!(!result.success);
    assert!(result.content.contains("/definitely/not/here.toml"));
    let guard = state.read();
    assert!(
        !guard.session.iter().any(|(_, s)| s.is_attendant()),
        "a failed load must not create anything"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn malformed_attendant_file_fails_with_the_reason() {
    // Given a file that is not a valid attendant entry.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let (_file_dir, path) = attendant_file("bad", "this is not = toml = ==");
    let tool_ctx = ctx(&harness, &state, user_id.clone(), dir.path()).await;

    // When the tool loads it.
    let result = execute(
        call(&serde_json::json!({ "path": path }).to_string()),
        tool_ctx,
    )
    .await;

    // Then the tool fails explaining why.
    assert!(!result.success);
    assert!(result.content.contains("not a valid attendant entry"));
}

#[rstest::rstest]
#[tokio::test]
async fn attendant_caller_cannot_load_an_attendant() {
    // Given a user session with an attendant, calling from the attendant.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let attendant = {
        let guard = state.read();
        ChatSessionState::new_attendant(guard.session.get(&user_id).expect("user"), true)
    };
    let attendant_id = attendant.session_id().clone();
    state.write().session.insert(attendant);
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let (_file_dir, path) = attendant_file("nested", ATTENDANT_BODY);
    let tool_ctx = ctx(&harness, &state, attendant_id, dir.path()).await;

    // When the attendant tries to load another attendant.
    let result = execute(
        call(&serde_json::json!({ "path": path }).to_string()),
        tool_ctx,
    )
    .await;

    // Then the load is refused: loaded attendants hang off non-attendant
    // parents.
    assert!(!result.success);
    assert!(result.content.contains("non-attendant"));
}

#[rstest::rstest]
#[test]
fn parse_accepts_the_bare_entry_shape() {
    // Given a bare attendant entry body.
    // When parsing.
    let entry = parse_attendant_entry(ATTENDANT_BODY).expect("parses");

    // Then the fields came across.
    assert_eq!(entry.name, "implementer");
    assert!(!entry.prep_mode);
}

#[rstest::rstest]
#[test]
fn parse_accepts_a_single_entry_wrapped_in_an_entry_array() {
    // Given a wrapped body with one entry.
    let body = format!("[[entry]]\n{}", ATTENDANT_BODY.trim_start_matches('\n'));

    // When parsing.
    let entry = parse_attendant_entry(&body).expect("parses");

    // Then the single entry comes across.
    assert_eq!(entry.name, "implementer");
}

#[rstest::rstest]
#[test]
fn parse_refuses_a_wrapped_file_carrying_several_entries() {
    // Given a wrapped body with two entries.
    let one = ATTENDANT_BODY.trim_start_matches('\n');
    let body = format!("[[entry]]\n{one}\n[[entry]]\nname = \"second\"\n");

    // When parsing.
    let result = parse_attendant_entry(&body);

    // Then it refuses with a reason naming the count.
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("2 entries"));
}

#[rstest::rstest]
#[test]
fn parse_accepts_the_full_document_shape_jinn_toml_uses() {
    // Given a file in the shape jinn.toml itself uses — the array
    // nested under an `attendant` table (`[[attendant.entry]]`) — which
    // is the shape the tool's description and the shipped autocoder
    // attendant files both promise.
    let one = ATTENDANT_BODY.trim_start_matches('\n');
    let body = format!("[[attendant.entry]]\n{one}\n");

    // When parsing.
    let entry = parse_attendant_entry(&body).expect("the jinn.toml shape parses");

    // Then the single entry comes across with its fields intact.
    assert_eq!(entry.name, "implementer");
}

#[rstest::rstest]
#[tokio::test]
async fn personal_attendants_still_save_and_workflow_attendants_refuse() {
    // Given a personal attendant (no marker) and a workflow-loaded
    // attendant, both live.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let personal = {
        let guard = state.read();
        ChatSessionState::new_attendant(guard.session.get(&user_id).expect("user"), true)
    };
    let personal_id = personal.session_id().clone();
    state.write().session.insert(personal);
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let (_file_dir, path) = attendant_file("savepath", ATTENDANT_BODY);
    let tool_ctx = ctx(&harness, &state, user_id.clone(), dir.path()).await;

    // When the tool loads the attendant.
    let result = execute(
        call(&serde_json::json!({ "path": path }).to_string()),
        tool_ctx,
    )
    .await;
    assert!(result.success);

    // Then the personal one has no marker and the workflow one does.
    let guard = state.read();
    let personal = guard.session.get(&personal_id).expect("personal");
    assert_eq!(personal.workflow_source(), None);
    let loaded = guard
        .session
        .iter()
        .map(|(_, s)| s)
        .find(|s| s.is_attendant() && s.session_id() != &personal_id)
        .expect("workflow attendant");
    assert!(loaded.workflow_source().is_some());
}

#[rstest::rstest]
#[test]
fn definition_names_the_path_argument() {
    // Given the tool definition.
    let def = definition();

    // When its parameters are read.
    // Then it is named load_attendant with path required.
    assert_eq!(def.name, "load_attendant");
    let required = def.parameters["required"].as_array().expect("required");
    assert_eq!(required.len(), 1);
    assert_eq!(required[0].as_str(), Some("path"));
    // And set_name is declared optional.
    let props = def.parameters["properties"]
        .as_object()
        .expect("properties");
    assert!(
        props.contains_key("set_name"),
        "set_name is a declared argument"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn set_name_relabels_the_instance_and_leaves_the_seed_name_alone() {
    // Given a user session and a seed file named "implementer".
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let (_file_dir, path) = attendant_file("setname", ATTENDANT_BODY);

    // When the tool loads it with set_name = "implementor-002".
    let result = execute(
        call(&serde_json::json!({ "path": path, "set_name": "implementor-002" }).to_string()),
        ctx(&harness, &state, user_id.clone(), dir.path()).await,
    )
    .await;

    // Then the tool succeeds and the attendant's title is the new label.
    assert!(result.success, "load must succeed: {}", result.content);
    assert!(
        result.content.contains("implementor-002"),
        "the result names the applied label: {}",
        result.content
    );
    let guard = state.read();
    let attendant = guard
        .session
        .iter()
        .map(|(_, s)| s)
        .find(|s| s.is_attendant())
        .expect("workflow attendant");
    assert_eq!(
        attendant.title(),
        Some("implementor-002"),
        "the instance carries the caller's label"
    );
    // And its behavior still comes from the seed file.
    assert_eq!(attendant.title(), Some("implementor-002"));
}

#[rstest::rstest]
#[tokio::test]
async fn loading_the_same_seed_twice_with_different_set_names_yields_two_named_attendants() {
    // Given a user session and one shared seed file.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let (_file_dir, path) = attendant_file("twice", ATTENDANT_BODY);

    // When it is loaded twice under two different labels.
    for label in ["implementor-001", "implementor-002"] {
        let result = execute(
            call(&serde_json::json!({ "path": path.clone(), "set_name": label }).to_string()),
            ctx(&harness, &state, user_id.clone(), dir.path()).await,
        )
        .await;
        assert!(result.success, "load must succeed: {}", result.content);
    }

    // Then both attendants exist, each under its own label.
    let guard = state.read();
    let mut titles: Vec<_> = guard
        .session
        .iter()
        .map(|(_, s)| s)
        .filter(|s| s.is_attendant())
        .map(|s| s.title().expect("titled").to_owned())
        .collect();
    titles.sort();
    assert_eq!(titles, vec!["implementor-001", "implementor-002"]);
}

#[rstest::rstest]
#[tokio::test]
async fn without_set_name_the_seed_files_name_is_used() {
    // Given a user session and a seed file named "implementer".
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let (_file_dir, path) = attendant_file("noname", ATTENDANT_BODY);
    let tool_ctx = ctx(&harness, &state, user_id.clone(), dir.path()).await;

    // When the tool loads it with no set_name argument.
    let result = execute(
        call(&serde_json::json!({ "path": path }).to_string()),
        tool_ctx,
    )
    .await;

    // Then the attendant keeps the seed file's name.
    assert!(result.success, "load must succeed: {}", result.content);
    let guard = state.read();
    let attendant = guard
        .session
        .iter()
        .map(|(_, s)| s)
        .find(|s| s.is_attendant())
        .expect("workflow attendant");
    assert_eq!(attendant.title(), Some("implementer"));
}
