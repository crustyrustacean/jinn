//! Tests for the `run_lifecycle_script` tool: hostile-argument quoting,
//! named→positional ordering, unknown-script refusal, and teardown
//! arg-stamping. The quoting assertions call the same render path the tool
//! calls, against the lifecycle crate's already-tested implementation.

#![allow(clippy::expect_used, reason = "test code")]

use std::collections::BTreeMap;
use std::time::Duration;

use jinn_common::app_paths::AppPaths;
use jinn_core_types::SessionId;
use jinn_core_types::tool_types::ToolCall;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::state::State;
use jinn_session_lifecycle_msg::command_template::CommandTemplate;
use jinn_session_lifecycle_msg::{RunSessionSetup, RunSessionTeardown};
use jinn_session_state::ChatSessionState;
use jinn_testutil::bus_harness::{TestHarness, await_recorded};

use crate::run_lifecycle_script::{execute, render_command};
use crate::tool_types::ToolContext;

/// Builds a tool call with the given JSON arguments.
fn call(arguments: &str) -> ToolCall {
    ToolCall {
        id: "tc_lc_1".to_owned(),
        name: "run_lifecycle_script".to_owned(),
        arguments: arguments.to_owned(),
    }
}

/// A config document carrying one shell lifecycle script with named args.
const SCRIPT_DOC: &str = r#"
[[session_lifecycle.script]]
name = "worktree"
description = "fork a worktree off a base branch"
setup_command = "mkdir <dir> && echo ./<dir> --branch <base>"
teardown_command = "fossil close && rm -rf <dir>"
"#;

/// Builds a tool context wired to the harness, with the script document.
async fn ctx(harness: &TestHarness, state: &State, session_id: SessionId) -> ToolContext {
    ToolContext {
        cwd: std::path::PathBuf::from("/tmp"),
        config: jinn_config::testutil::config_layer(SCRIPT_DOC),
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

/// Seeds one plain session; returns (state, session_id).
fn fixture() -> (State, SessionId) {
    let state = State::new(AppState::default_with_scope_focus());
    let session = ChatSessionState::new();
    let id = session.session_id().clone();
    state.write().session.insert(session);
    (state, id)
}

#[rstest::rstest]
#[test]
fn hostile_argument_values_cannot_restructure_the_command() {
    // Given values carrying spaces, quotes, dollar signs, and separators.
    let args = BTreeMap::from([
        ("dir".to_owned(), "feat x; rm -rf $HOME".to_owned()),
        ("base".to_owned(), "it's $(pwd) `back`".to_owned()),
    ]);

    // When rendering the script's setup command.
    let rendered =
        render_command("mkdir <dir> && echo ./<dir> --branch <base>", &args).expect("renders");

    // Then every hostile value landed inside single quotes: the command's
    // structure (`&&`, words) is exactly the script author's, and no value
    // contributed a separator, expansion, or quote of its own.
    assert!(rendered.contains("'feat x; rm -rf $HOME'"), "{rendered}");
    assert!(rendered.contains("'it'\\''s $(pwd) `back`'"), "{rendered}");
    assert_eq!(
        rendered.matches("&&").count(),
        1,
        "the only && is the script's own: {rendered}"
    );
}

#[rstest::rstest]
#[test]
fn a_value_containing_a_placeholder_shape_is_not_re_substituted() {
    // Given a value that looks like a placeholder.
    let args = BTreeMap::from([
        ("dir".to_owned(), "<base>".to_owned()),
        ("base".to_owned(), "trunk".to_owned()),
    ]);

    // When rendering.
    let rendered = render_command("echo <dir> <base>", &args).expect("renders");

    // Then the placeholder-shaped value was quoted literally, not swapped
    // for the other argument: render substitutes in one pass.
    assert!(rendered.contains("'<base>'"), "{rendered}");
    assert!(rendered.contains("trunk"), "{rendered}");
}

#[rstest::rstest]
#[test]
fn missing_argument_fails_naming_the_placeholder() {
    // Given args missing one of the script's placeholders.
    let args = BTreeMap::from([("base".to_owned(), "trunk".to_owned())]);

    // When rendering.
    let result = render_command("mkdir <dir> --branch <base>", &args);

    // Then it fails naming <dir>.
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("<dir>"));
}

#[rstest::rstest]
#[test]
fn ordering_places_named_args_in_template_parameter_order() {
    // Given args supplied in an order different from the template's.
    let args = BTreeMap::from([
        ("base".to_owned(), "trunk".to_owned()),
        ("dir".to_owned(), "wt".to_owned()),
    ]);

    // When ordering against the template (dir first, base second).
    let template = CommandTemplate::parse("mkdir <dir> --branch <base>");
    let ordered = crate::run_lifecycle_script::order_args(&template, &args);

    // Then the values come out positionally.
    assert_eq!(ordered, vec!["wt".to_owned(), "trunk".to_owned()]);
}

#[rstest::rstest]
#[tokio::test]
async fn setup_publishes_the_rendered_command_and_awaits_completion() {
    // Given a session and a harness recording lifecycle commands.
    let harness = TestHarness::new().await;
    let setups = harness.spawn_recorder::<RunSessionSetup>().await;
    let (state, id) = fixture();
    let tool_ctx = ctx(&harness, &state, id.clone()).await;

    // When the tool runs setup — racing it against our own completion
    // publish, since no lifecycle actor exists in this harness to answer.
    let tool = tokio::spawn(execute(
        call(&format!(
            r#"{{"script":"worktree","action":"setup","session_id":"{id}","args":{{"dir":"wt-a","base":"trunk"}}}}"#
        )),
        tool_ctx,
    ));
    // Give the listener a moment to subscribe, then answer as the actor
    // would.
    tokio::time::sleep(Duration::from_millis(150)).await;
    harness
        .publish(jinn_session_lifecycle_msg::SessionSetupCompleted {
            session_id: id.clone(),
            cwd: std::path::PathBuf::from("/repo/wt-a"),
            error: None,
        })
        .await;
    let result = tool.await.expect("tool task");

    // Then the tool succeeded and the published command carries the
    // rendered, quoted command with ordered args.
    assert!(result.success, "{}", result.content);
    let recorded = await_recorded::<RunSessionSetup>(&setups, 1, Duration::from_secs(2)).await;
    assert_eq!(recorded[0].session_id, id);
    // Safe values pass through unquoted (shell_quote quotes only specials).
    assert!(
        recorded[0].command.contains("mkdir wt-a"),
        "{}",
        recorded[0].command
    );
    assert!(
        recorded[0].command.contains("--branch trunk"),
        "{}",
        recorded[0].command
    );
    assert_eq!(
        recorded[0].args,
        vec!["wt-a".to_owned(), "trunk".to_owned()]
    );
    // And the tool reported the adopted cwd.
    assert!(result.content.contains("/repo/wt-a"), "{}", result.content);
}

#[rstest::rstest]
#[tokio::test]
async fn teardown_stamps_lifecycle_name_and_args_before_publishing() {
    // Given a session and a harness recording teardown commands.
    let harness = TestHarness::new().await;
    let teardowns = harness.spawn_recorder::<RunSessionTeardown>().await;
    let (state, id) = fixture();
    let tool_ctx = ctx(&harness, &state, id.clone()).await;

    // When the tool runs teardown, answered by the event the actor would
    // publish.
    let tool = tokio::spawn(execute(
        call(&format!(
            r#"{{"script":"worktree","action":"teardown","session_id":"{id}","args":{{"dir":"wt-a","base":"trunk"}}}}"#
        )),
        tool_ctx,
    ));
    tokio::time::sleep(Duration::from_millis(150)).await;
    harness
        .publish(jinn_session_lifecycle_msg::SessionTeardownFinished {
            session_id: id.clone(),
            error: None,
        })
        .await;
    let result = tool.await.expect("tool task");

    // Then the session now carries the script name and ordered args — the
    // actor re-renders teardown from these, so they must be stamped.
    assert!(result.success, "{}", result.content);
    let guard = state.read();
    let session = guard.session.get(&id).expect("session");
    assert_eq!(session.lifecycle_name(), Some("worktree"));
    // The teardown template only names <dir>, so the stored args carry
    // exactly what the teardown replay will substitute.
    assert_eq!(session.lifecycle_args(), &["wt-a".to_owned()]);
    // And the published command named the script's teardown template.
    drop(guard);
    let recorded =
        await_recorded::<RunSessionTeardown>(&teardowns, 1, Duration::from_secs(2)).await;
    assert!(
        recorded[0].command.contains("rm -rf"),
        "{}",
        recorded[0].command
    );
}

#[rstest::rstest]
#[tokio::test]
async fn unknown_script_fails_naming_the_known_ones() {
    // Given a state with the worktree script configured.
    let harness = TestHarness::new().await;
    let (state, id) = fixture();
    let tool_ctx = ctx(&harness, &state, id.clone()).await;

    // When the tool runs an unknown script name.
    let result = execute(
        call(&format!(
            r#"{{"script":"nope","action":"setup","session_id":"{id}","args":{{}}}}"#
        )),
        tool_ctx,
    )
    .await;

    // Then it fails naming the miss and the known scripts.
    assert!(!result.success);
    assert!(result.content.contains("unknown lifecycle script"));
    assert!(result.content.contains("worktree"));
}

#[rstest::rstest]
#[tokio::test]
async fn missing_placeholder_argument_fails_before_any_publish() {
    // Given a harness recording setup commands.
    let harness = TestHarness::new().await;
    let setups = harness.spawn_recorder::<RunSessionSetup>().await;
    let (state, id) = fixture();
    let tool_ctx = ctx(&harness, &state, id.clone()).await;

    // When the tool runs with only one of the two placeholders supplied.
    let result = execute(
        call(&format!(
            r#"{{"script":"worktree","action":"setup","session_id":"{id}","args":{{"dir":"wt"}}}}"#
        )),
        tool_ctx,
    )
    .await;

    // Then it fails naming <base> and publishes nothing.
    assert!(!result.success);
    assert!(result.content.contains("<base>"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(setups.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn a_failed_script_reports_the_failure_as_a_failed_result() {
    // Given a session whose script will report an error.
    let harness = TestHarness::new().await;
    let (state, id) = fixture();
    let tool_ctx = ctx(&harness, &state, id.clone()).await;

    // When the tool runs and the completion carries an error.
    let tool = tokio::spawn(execute(
        call(&format!(
            r#"{{"script":"worktree","action":"setup","session_id":"{id}","args":{{"dir":"wt","base":"trunk"}}}}"#
        )),
        tool_ctx,
    ));
    tokio::time::sleep(Duration::from_millis(150)).await;
    harness
        .publish(jinn_session_lifecycle_msg::SessionSetupCompleted {
            session_id: id.clone(),
            cwd: std::path::PathBuf::from("/tmp"),
            error: Some("exit code 1".to_owned()),
        })
        .await;
    let result = tool.await.expect("tool task");

    // Then the tool result fails and carries the script's error.
    assert!(!result.success);
    assert!(result.content.contains("exit code 1"), "{}", result.content);
}
