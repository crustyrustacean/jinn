//! Actor-level tests for the `restart_mcp_server` tool.
//!
//! These bypass the coordinator entirely: they construct a `ToolContext` with a
//! real bus, call `execute()` directly, and drive the four outcomes by manually
//! publishing `McpServerStatus` events (Starting/Running/Dead). This isolates
//! the load-bearing logic — wait-for-terminal + Gotcha #3 ordering guard +
//! timeout — without needing a live MCP server.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test assertions"
)]

use std::path::PathBuf;
use std::time::Duration;

use crate::common::app_paths::AppPaths;
use crate::common::app_state::AppState;
use crate::common::bus::test_harness::TestHarness;
use crate::common::state::State;
use crate::feat::mcp::McpServerConfig;
use crate::feat::mcp_actor::protocol::{McpConnectionStatus, McpServerStatus};
use crate::feat::preferences_actor::UserPreferences;
use crate::feat::tools_actor::restart_mcp::{RESTART_TIMEOUT, execute_with_timeout};
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext};
use crate::feat::ui::frontend_state::FrontendState;
use crate::protocol::SessionId;

/// A short timeout for tests so they run fast.
const TEST_TIMEOUT: Duration = Duration::from_millis(300);

/// Builds a tool call targeting the given server.
fn call(server: &str) -> ToolCall {
    ToolCall {
        id: "tc_1".to_owned(),
        name: "restart_mcp_server".to_owned(),
        arguments: format!("{{\"server\": \"{server}\"}}"),
    }
}

/// Builds a ToolContext wired to the harness bus and a state that has
/// `excalimate` configured.
fn ctx_with_excalimate(
    harness_bus: crate::common::services::bus_service::BusService,
    session_id: SessionId,
) -> ToolContext {
    let config = McpServerConfig {
        name: "excalimate".to_owned(),
        command: String::new(),
        args: vec![],
        ..Default::default()
    };
    let frontend = FrontendState {
        preferences: UserPreferences {
            mcp_servers: vec![config],
            ..Default::default()
        },
        ..Default::default()
    };
    let app = AppState {
        frontend,
        ..Default::default()
    };
    let state = State::new(app);

    ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        state: Some(state),
        session_id: Some(session_id),
        app_paths: AppPaths::new_in(std::path::Path::new("/tmp")),
        bus: Some(harness_bus),
        max_output_lines: None,
        max_output_bytes: None,
        dispatched_at: jiff::Timestamp::now(),
        session_cap: None,
    }
}

fn status(session_id: &SessionId, server: &str, s: McpConnectionStatus) -> McpServerStatus {
    McpServerStatus {
        session_id: session_id.clone(),
        server: server.to_owned(),
        status: s,
    }
}

/// The new server reaches `Running` after `Starting` → the tool reports success.
#[tokio::test]
async fn execute_returns_success_when_new_server_reaches_running() {
    // Given a harness + a context with `excalimate` configured.
    let harness = TestHarness::new().await;
    let session_id = SessionId::new();
    let ctx = ctx_with_excalimate(harness.bus(), session_id.clone());

    // When calling execute and driving Starting then Running on the bus.
    let task = tokio::spawn(execute_with_timeout(
        call("excalimate"),
        ctx,
        RESTART_TIMEOUT,
    ));
    // Let the tool subscribe + publish RestartMcpServer.
    tokio::time::sleep(Duration::from_millis(120)).await;
    harness
        .publish(status(
            &session_id,
            "excalimate",
            McpConnectionStatus::Starting,
        ))
        .await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    harness
        .publish(status(
            &session_id,
            "excalimate",
            McpConnectionStatus::Running,
        ))
        .await;

    let result = task.await.expect("task");

    // Then the tool reports success.
    assert!(
        result.success,
        "should succeed on Running; got: {}",
        result.content
    );
    assert!(
        result.content.contains("back online"),
        "success message should mention back online; got: {}",
        result.content
    );
}

/// The new server goes `Dead` → the tool reports failure with the STOP note.
#[tokio::test]
async fn execute_returns_failure_with_stop_note_when_new_server_goes_dead() {
    // Given a harness + context.
    let harness = TestHarness::new().await;
    let session_id = SessionId::new();
    let ctx = ctx_with_excalimate(harness.bus(), session_id.clone());

    // When calling execute and driving Starting then Dead.
    let task = tokio::spawn(execute_with_timeout(
        call("excalimate"),
        ctx,
        RESTART_TIMEOUT,
    ));
    tokio::time::sleep(Duration::from_millis(120)).await;
    harness
        .publish(status(
            &session_id,
            "excalimate",
            McpConnectionStatus::Starting,
        ))
        .await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    harness
        .publish(status(&session_id, "excalimate", McpConnectionStatus::Dead))
        .await;

    let result = task.await.expect("task");

    // Then the tool reports failure with the STOP-and-wait instruction.
    assert!(
        !result.success,
        "should fail on Dead; got: {}",
        result.content
    );
    assert!(
        result.content.contains("STOP"),
        "failure message should include the STOP instruction; got: {}",
        result.content
    );
}

/// The tool does NOT return while the status is only `Starting`.
#[tokio::test]
async fn execute_does_not_return_while_status_is_starting() {
    // Given a harness + context.
    let harness = TestHarness::new().await;
    let session_id = SessionId::new();
    let ctx = ctx_with_excalimate(harness.bus(), session_id.clone());

    // When calling execute and driving only Starting.
    let task = tokio::spawn(execute_with_timeout(
        call("excalimate"),
        ctx,
        RESTART_TIMEOUT,
    ));
    tokio::time::sleep(Duration::from_millis(120)).await;
    harness
        .publish(status(
            &session_id,
            "excalimate",
            McpConnectionStatus::Starting,
        ))
        .await;

    // Then the future is still pending after a generous wait (no terminal
    // status has arrived).
    let pending = tokio::select! {
        _r = task => "resolved",
        () = tokio::time::sleep(Duration::from_millis(200)) => "pending",
    };
    assert_eq!(pending, "pending", "tool must not return mid-startup");
}

/// The Gotcha #3 guard: a stale Dead from the old actor (before the new
/// Starting) is ignored, not treated as the new server's failure.
#[tokio::test]
async fn execute_ignores_stale_dead_from_old_actor_before_new_starting() {
    // Given a harness + context.
    let harness = TestHarness::new().await;
    let session_id = SessionId::new();
    let ctx = ctx_with_excalimate(harness.bus(), session_id.clone());

    // When calling execute and driving (stale)Dead -> (new)Starting -> Running.
    let task = tokio::spawn(execute_with_timeout(
        call("excalimate"),
        ctx,
        RESTART_TIMEOUT,
    ));
    tokio::time::sleep(Duration::from_millis(120)).await;
    // Stale Dead from the old actor's teardown (arrives first).
    harness
        .publish(status(&session_id, "excalimate", McpConnectionStatus::Dead))
        .await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    harness
        .publish(status(
            &session_id,
            "excalimate",
            McpConnectionStatus::Starting,
        ))
        .await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    harness
        .publish(status(
            &session_id,
            "excalimate",
            McpConnectionStatus::Running,
        ))
        .await;

    let result = task.await.expect("task");

    // Then the tool reports success — the stale Dead was correctly ignored.
    assert!(
        result.success,
        "stale pre-Starting Dead must be ignored; got: {}",
        result.content
    );
}

/// A never-Running server → after the timeout, the tool reports "still
/// starting" (success=true), not a hard failure.
#[tokio::test]
async fn execute_returns_still_starting_on_timeout() {
    // Given a harness + context, with a short test timeout.
    let harness = TestHarness::new().await;
    let session_id = SessionId::new();
    let ctx = ctx_with_excalimate(harness.bus(), session_id.clone());

    // When calling execute with a short timeout and driving only Starting
    // (no terminal status ever arrives in phase 1; phase 2 times out).
    let task = tokio::spawn(execute_with_timeout(call("excalimate"), ctx, TEST_TIMEOUT));
    tokio::time::sleep(Duration::from_millis(120)).await;
    harness
        .publish(status(
            &session_id,
            "excalimate",
            McpConnectionStatus::Starting,
        ))
        .await;

    let result = task.await.expect("task");

    // Then the tool reports "still starting" with success=true.
    assert!(
        result.success,
        "timeout should not be a hard failure; got: {}",
        result.content
    );
    assert!(
        result.content.contains("still starting"),
        "should report still-starting on timeout; got: {}",
        result.content
    );
}

/// An unknown server name → clear failure, no restart attempted.
#[tokio::test]
async fn execute_returns_failure_for_unknown_server() {
    // Given a context with only `excalimate` configured.
    let harness = TestHarness::new().await;
    let session_id = SessionId::new();
    let ctx = ctx_with_excalimate(harness.bus(), session_id);

    // When calling execute with an unconfigured server name.
    let result = execute_with_timeout(call("ghost"), ctx, RESTART_TIMEOUT).await;

    // Then the tool fails fast with the server name in the message.
    assert!(!result.success, "unknown server should fail");
    assert!(
        result.content.contains("ghost"),
        "error should name the unknown server; got: {}",
        result.content
    );
}

/// A `mcp__<server>__<tool>` tool name is silently resolved to the server.
#[tokio::test]
async fn execute_silently_strips_namespace_from_tool_name() {
    // Given a harness + context with `excalimate` configured.
    let harness = TestHarness::new().await;
    let session_id = SessionId::new();
    let ctx = ctx_with_excalimate(harness.bus(), session_id.clone());

    // When calling execute with a namespaced tool name and driving Running.
    let task = tokio::spawn(execute_with_timeout(
        ToolCall {
            id: "tc_1".to_owned(),
            name: "restart_mcp_server".to_owned(),
            arguments: "{\"server\": \"mcp__excalimate__create_scene\"}".to_owned(),
        },
        ctx,
        RESTART_TIMEOUT,
    ));
    tokio::time::sleep(Duration::from_millis(120)).await;
    harness
        .publish(status(
            &session_id,
            "excalimate",
            McpConnectionStatus::Starting,
        ))
        .await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    harness
        .publish(status(
            &session_id,
            "excalimate",
            McpConnectionStatus::Running,
        ))
        .await;

    let result = task.await.expect("task");

    // Then the tool succeeds — the namespace was stripped and the server matched.
    assert!(
        result.success,
        "namespaced name should resolve to the server; got: {}",
        result.content
    );
}
