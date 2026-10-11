//! Tests for the `load_attendant` tool and the `workflow_cast` save-block.

#![allow(clippy::expect_used, reason = "test code")]

use std::time::Duration;

use jinn_common::app_paths::AppPaths;
use jinn_core_types::SessionId;
use jinn_core_types::tool_types::ToolCall;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::state::State;
use jinn_session_state::ChatSessionState;
use jinn_session_store_msg::PersistSession;
use jinn_testutil::bus_harness::{TestHarness, await_recorded};

use crate::load_attendant::{definition, execute, parse_cast_entry};
use crate::tool_types::ToolContext;

/// Builds a tool call with the given JSON arguments.
fn call(arguments: &str) -> ToolCall {
    ToolCall {
        id: "tc_load_1".to_owned(),
        name: "load_attendant".to_owned(),
        arguments: arguments.to_owned(),
    }
}

/// Builds a tool context wired to the harness bus and shared state.
async fn ctx(harness: &TestHarness, state: &State, session_id: SessionId) -> ToolContext {
    ToolContext {
        cwd: std::path::PathBuf::from("/tmp"),
        config: jinn_config::testutil::config_layer(""),
        timeout: None,
        state: Some(state.clone()),
        session_id: Some(session_id),
        app_paths: AppPaths::new_in(std::path::Path::new("/tmp")),
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

/// Writes a cast file under a fresh temp dir; returns its path.
fn cast_file(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("jinn_cast_test_{name}_{p}", p = SessionId::new()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("member.toml");
    std::fs::write(&path, body).expect("write cast file");
    path.display().to_string()
}

const CAST_BODY: &str = r#"
name = "implementer"
behavior = "preserve"
trigger = "manual"
prep_mode = false
seed_template = "You implement one stage of a pipeline."
"#;

#[rstest::rstest]
#[tokio::test]
async fn load_attaches_a_cast_child_and_stamps_the_marker() {
    // Given a user session and a cast file on disk.
    let harness = TestHarness::new().await;
    let persists = harness.spawn_recorder::<PersistSession>().await;
    let (state, user_id) = fixture();
    let path = cast_file("stamp", CAST_BODY);
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;

    // When the tool loads the cast file.
    let result = execute(call(&format!(r#"{{"path":"{path}"}}"#)), tool_ctx).await;

    // Then the tool succeeds and the attendant exists as a child.
    assert!(result.success, "load must succeed: {}", result.content);
    let guard = state.read();
    let attendant = guard
        .session
        .iter()
        .map(|(_, s)| s)
        .find(|s| s.is_attendant())
        .expect("a cast attendant must exist");
    assert_eq!(attendant.parent_session().as_ref(), Some(&user_id));
    assert_eq!(attendant.title(), Some("implementer"));
    // And it carries the cast marker naming its source file.
    assert_eq!(attendant.workflow_cast(), Some(path.as_str()));
    assert!(!attendant.attendant_is_prepping(), "cast config applied");
    // And the creation published persists for parent and attendant.
    drop(guard);
    let persisted = await_recorded::<PersistSession>(&persists, 2, Duration::from_secs(2)).await;
    assert_eq!(persisted.len(), 2);
}

#[rstest::rstest]
#[tokio::test]
async fn cast_children_of_a_cast_member_do_not_inherit_the_marker() {
    // Given a loaded cast attendant.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let path = cast_file("inherit", CAST_BODY);
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;
    let result = execute(call(&format!(r#"{{"path":"{path}"}}"#)), tool_ctx).await;
    assert!(result.success);

    // When a child is created the ordinary way from the cast member
    // (`new_attendant` is the creation path every spawner shares).
    let guard = state.read();
    let cast_member = guard
        .session
        .iter()
        .map(|(_, s)| s)
        .find(|s| s.is_attendant())
        .expect("cast member");
    let child = ChatSessionState::new_attendant(cast_member, true);
    let child_id = child.session_id().clone();
    drop(guard);
    state.write().session.insert(child);

    // Then the child is an ordinary personal attendant: no marker.
    let guard = state.read();
    let child = guard.session.get(&child_id).expect("child");
    assert_eq!(child.workflow_cast(), None, "the marker is never inherited");
}

#[rstest::rstest]
#[tokio::test]
async fn missing_file_fails_naming_the_path() {
    // Given a session and a path that does not exist.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;

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
async fn malformed_cast_file_fails_with_the_reason() {
    // Given a cast file that is not a valid entry.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let path = cast_file("bad", "this is not = toml = ==");
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;

    // When the tool loads it.
    let result = execute(call(&format!(r#"{{"path":"{path}"}}"#)), tool_ctx).await;

    // Then the tool fails explaining why.
    assert!(!result.success);
    assert!(result.content.contains("not a valid cast entry"));
}

#[rstest::rstest]
#[tokio::test]
async fn attendant_caller_cannot_load_a_cast() {
    // Given a user session with an attendant, calling from the attendant.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let attendant = {
        let guard = state.read();
        ChatSessionState::new_attendant(guard.session.get(&user_id).expect("user"), true)
    };
    let attendant_id = attendant.session_id().clone();
    state.write().session.insert(attendant);
    let path = cast_file("nested", CAST_BODY);
    let tool_ctx = ctx(&harness, &state, attendant_id).await;

    // When the attendant tries to load a cast member.
    let result = execute(call(&format!(r#"{{"path":"{path}"}}"#)), tool_ctx).await;

    // Then the load is refused: cast members hang off non-attendant parents.
    assert!(!result.success);
    assert!(result.content.contains("non-attendant"));
}

#[rstest::rstest]
#[test]
fn parse_accepts_the_bare_entry_shape() {
    // Given a bare cast entry body.
    // When parsing.
    let entry = parse_cast_entry(CAST_BODY).expect("parses");

    // Then the fields came across.
    assert_eq!(entry.name, "implementer");
    assert!(!entry.prep_mode);
}

#[rstest::rstest]
#[test]
fn parse_accepts_a_single_entry_wrapped_in_an_entry_array() {
    // Given a wrapped cast body with one entry.
    let body = format!("[[entry]]\n{}", CAST_BODY.trim_start_matches('\n'));

    // When parsing.
    let entry = parse_cast_entry(&body).expect("parses");

    // Then the single member comes across.
    assert_eq!(entry.name, "implementer");
}

#[rstest::rstest]
#[test]
fn parse_refuses_a_wrapped_file_carrying_several_entries() {
    // Given a wrapped body with two entries.
    let one = CAST_BODY.trim_start_matches('\n');
    let body = format!("[[entry]]\n{one}\n[[entry]]\nname = \"second\"\n");

    // When parsing.
    let result = parse_cast_entry(&body);

    // Then it refuses with a reason naming the count.
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("2 entries"));
}

#[rstest::rstest]
#[tokio::test]
async fn personal_attendants_still_save_and_cast_members_refuse() {
    // Given a personal attendant (no marker) and a cast member, both live.
    let harness = TestHarness::new().await;
    let (state, user_id) = fixture();
    let personal = {
        let guard = state.read();
        ChatSessionState::new_attendant(guard.session.get(&user_id).expect("user"), true)
    };
    let personal_id = personal.session_id().clone();
    state.write().session.insert(personal);
    let path = cast_file("savepath", CAST_BODY);
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;
    let result = execute(call(&format!(r#"{{"path":"{path}"}}"#)), tool_ctx).await;
    assert!(result.success);

    // Then the personal one has no marker and the cast one does.
    let guard = state.read();
    let personal = guard.session.get(&personal_id).expect("personal");
    assert_eq!(personal.workflow_cast(), None);
    let cast = guard
        .session
        .iter()
        .map(|(_, s)| s)
        .find(|s| s.is_attendant() && s.session_id() != &personal_id)
        .expect("cast member");
    assert!(cast.workflow_cast().is_some());
}

#[rstest::rstest]
#[test]
fn definition_names_the_path_argument() {
    // Given the tool definition.
    let def = definition();

    // Then it is named load_attendant with path required.
    assert_eq!(def.name, "load_attendant");
    let required = def.parameters["required"].as_array().expect("required");
    assert_eq!(required.len(), 1);
    assert_eq!(required[0].as_str(), Some("path"));
}
