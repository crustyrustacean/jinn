//! Minigraf dispatch interception: the session's active graph rides on every
//! model-facing minigraf call, and the binding is written only from activate
//! results.
//!
//! These drive the real delivery path — `ExecuteToolBatch` published on the
//! trouper bus, the orchestrator resolving a registered `mcp__minigraf__*`
//! tool, and the published command or result coming back — because the
//! contract is about what actually reaches the server and what the session's
//! state ends up holding.
//!
//! The interception is jinn-only and argument-level: the model never sees a
//! `graph`/`created_by` parameter (the advertised schema is stripped at
//! registration), a call with no active binding is refused locally before any
//! `ExecuteTool` is published, and a successful activate result writes the
//! session's blob.

use std::time::Duration;

use jinn_core_types::tool_types::{ToolCall, ToolDefinition, ToolResult};
use jinn_kernel::common::bus::HarnessServices;
use jinn_kernel::common::state::State;
use jinn_mcp_msg::McpConnectionStatus;
use jinn_session_state::read_active_graph;
use jinn_testutil::bus_harness::{TestHarness, await_recorded};
use jinn_tools_msg::{ExecuteTool, ExecuteToolBatch, RegisterTools, ToolExecutionCompleted};

use crate::orchestrator::{ToolOrchestratorActor, ToolOrchestratorActorDeps};

const PROVIDER: &str = "mcp__minigraf__";

use jinn_core_types::SessionId;

/// Seeds a session into `state` with the minigraf server enabled and Running.
fn seed_session(state: &State, session_id: &SessionId) {
    state
        .write()
        .session
        .get_or_create(session_id)
        .enable_mcp_server("minigraf");
}

/// Marks the minigraf connection Running and seeds the singleton config so
/// the gate resolves its status from the singleton bucket.
async fn seed_running_connection(services: &jinn_kernel::common::services::Services) {
    services
        .config
        .put::<jinn_preferences_config::schemas::McpServersConfig>(
            &jinn_preferences_config::schemas::McpServersConfig(
                std::iter::once((
                    "minigraf".to_owned(),
                    jinn_preferences_config::schemas::mcp::McpServerConfig {
                        singleton: true,
                        ..Default::default()
                    },
                ))
                .collect(),
            ),
        )
        .expect("seed the mcp section");
    let runtime = services
        .slices
        .reader::<jinn_mcp_msg::McpRuntimeState>(&jinn_mcp_msg::mcp_runtime_slot())
        .expect("MCP runtime cell");
    runtime.update(|runtime| {
        runtime.set_singleton_status("minigraf", McpConnectionStatus::Running);
    });
}

/// Publishes one minigraf tool registration through the real path.
async fn register_minigraf_tool(harness: &TestHarness, tool_name: &str) {
    harness
        .publish(RegisterTools {
            provider: PROVIDER.to_owned(),
            definitions: vec![ToolDefinition {
                name: tool_name.to_owned(),
                description: String::new(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "graph": {"type": "string"},
                        "created_by": {"type": "string"}
                    },
                    "required": ["graph"]
                }),
                prompt_snippet: None,
                prompt_guidelines: vec![],
                server_tool_type: None,
            }],
            session_id: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(80)).await;
}

async fn spawn_orchestrator(harness: &TestHarness, state: State) {
    let services = harness.services().await;
    seed_running_connection(&services).await;
    ToolOrchestratorActor::spawn(
        harness.system(),
        ToolOrchestratorActorDeps {
            deps: jinn_kernel::common::actor_deps::ActorDeps {
                services: services.clone(),
            },
            state,
            services,
            builtin_filter: None,
        },
    );
}

fn call(name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: "tc-1".to_owned(),
        name: name.to_owned(),
        arguments: arguments.to_owned(),
    }
}

#[rstest::rstest]
#[tokio::test]
async fn transact_without_an_active_graph_is_rejected_locally_without_publishing() {
    // Given a session with minigraf running but no active graph bound.
    let state = State::new(jinn_kernel::AppState::default());
    let session_id = SessionId::new();
    seed_session(&state, &session_id);
    let harness = TestHarness::new().await;
    spawn_orchestrator(&harness, state).await;
    register_minigraf_tool(&harness, "mcp__minigraf__transact").await;
    let dispatched = harness.spawn_recorder::<ExecuteTool>().await;
    let completed = harness.spawn_recorder::<ToolExecutionCompleted>().await;

    // When the model transacts with no graph argument at all.
    harness
        .publish(ExecuteToolBatch {
            session_id: session_id.clone(),
            tool_calls: vec![call(
                "mcp__minigraf__transact",
                r#"{"datalog":"(transact [[:a :b 1]])"}"#,
            )],
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;

    // Then the refusal is local: a failed ToolExecutionCompleted arrives...
    let results =
        await_recorded::<ToolExecutionCompleted>(&completed, 1, Duration::from_secs(3)).await;
    assert!(!results[0].result.success, "must refuse without a binding");
    assert!(
        results[0].result.content.contains("instantiate"),
        "refusal names the remedy: {}",
        results[0].result.content
    );
    // ...and no ExecuteTool ever reaches the server.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        await_recorded::<ExecuteTool>(&dispatched, 0, Duration::from_millis(50))
            .await
            .is_empty(),
        "no ExecuteTool may be published for a locally rejected call"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn transact_injects_the_sessions_active_graph_name() {
    // Given a session whose active graph is bound in its blob.
    let state = State::new(jinn_kernel::AppState::default());
    let session_id = SessionId::new();
    seed_session(&state, &session_id);
    state.write().session.get_mut(&session_id).expect("session").blobs_mut().insert(
        jinn_session_state::ACTIVE_GRAPH_BLOB_KEY.to_owned(),
        serde_json::json!({"name": "stoplight-a", "template": "stoplight", "created_by": "other"}),
    );
    let harness = TestHarness::new().await;
    spawn_orchestrator(&harness, state).await;
    register_minigraf_tool(&harness, "mcp__minigraf__transact").await;
    let dispatched = harness.spawn_recorder::<ExecuteTool>().await;

    // When the model transacts (bare datalog, no graph argument).
    harness
        .publish(ExecuteToolBatch {
            session_id: session_id.clone(),
            tool_calls: vec![call(
                "mcp__minigraf__transact",
                r#"{"datalog":"(transact [[:a :b 1]])"}"#,
            )],
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;

    // Then the published ExecuteTool's arguments carry the bound graph name.
    let messages = await_recorded::<ExecuteTool>(&dispatched, 1, Duration::from_secs(3)).await;
    let args: serde_json::Value =
        serde_json::from_str(&messages[0].tool_call.arguments).expect("json args");
    assert_eq!(
        args.get("graph").and_then(serde_json::Value::as_str),
        Some("stoplight-a"),
        "graph must be injected: {args}"
    );
    assert!(
        args.get("created_by").is_none(),
        "transact carries no created_by: {args}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn activate_result_writes_the_active_graph_blob() {
    // Given a session with no active graph, the orchestrator running, and an
    // instantiate dispatch that armed the binding.
    let state = State::new(jinn_kernel::AppState::default());
    let session_id = SessionId::new();
    seed_session(&state, &session_id);
    let harness = TestHarness::new().await;
    spawn_orchestrator(&harness, state.clone()).await;
    register_minigraf_tool(&harness, "mcp__minigraf__instantiate_graph").await;
    harness
        .publish(ExecuteToolBatch {
            session_id: session_id.clone(),
            tool_calls: vec![call(
                "mcp__minigraf__instantiate_graph",
                r#"{"template":"stoplight","name":"stoplight-a"}"#,
            )],
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    // When the successful result for that call comes back through the bus
    // (what the minigraf client actor publishes on success).
    harness
        .publish(ToolExecutionCompleted {
            session_id: session_id.clone(),
            result: ToolResult {
                tool_call_id: "tc-1".to_owned(),
                name: "mcp__minigraf__instantiate_graph".to_owned(),
                content: "activated graph `stoplight-a` (instantiated from template \
                          `stoplight`, created by someone)\nactive graph: `stoplight-a`"
                    .to_owned(),
                success: true,
                full_content: None,
                truncation: None,
                pin_position: None,
            },
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Then the session's blob holds the binding with the template.
    let binding = {
        let guard = state.read();
        read_active_graph(
            &guard
                .session
                .get(&session_id)
                .expect("session exists")
                .blobs(),
        )
    };
    let binding = binding.expect("binding written from the activate result");
    assert_eq!(binding.name, "stoplight-a");
    assert_eq!(binding.template.as_deref(), Some("stoplight"));
}

#[rstest::rstest]
#[tokio::test]
async fn activate_calls_inject_the_calling_session_id() {
    // Given a session with the minigraf connection running.
    let state = State::new(jinn_kernel::AppState::default());
    let session_id = SessionId::new();
    seed_session(&state, &session_id);
    let harness = TestHarness::new().await;
    spawn_orchestrator(&harness, state).await;
    register_minigraf_tool(&harness, "mcp__minigraf__attach_graph").await;
    let dispatched = harness.spawn_recorder::<ExecuteTool>().await;

    // When the model attaches to an existing graph.
    harness
        .publish(ExecuteToolBatch {
            session_id: session_id.clone(),
            tool_calls: vec![call("mcp__minigraf__attach_graph", r#"{"name":"shared"}"#)],
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;

    // Then the published arguments carry the calling session's id as
    // created_by — the model has no such parameter to pass.
    let messages = await_recorded::<ExecuteTool>(&dispatched, 1, Duration::from_secs(3)).await;
    let args: serde_json::Value =
        serde_json::from_str(&messages[0].tool_call.arguments).expect("json args");
    assert_eq!(
        args.get("created_by").and_then(serde_json::Value::as_str),
        Some(session_id.to_string().as_str()),
        "created_by must be the calling session: {args}"
    );
}
