//! End-to-end fabric verification: orchestrator completes a real builtin tool
//! batch and the session actor continues the tool loop.
//!
//! The "MCP tools never returning" bug class was caused by tool-completion
//! messages lost on the pre-trouper fabric. These tests drive the REAL
//! delivery path — `ExecuteToolBatch` published on the trouper bus, the
//! spawned orchestrator executing a builtin, `ToolBatchCompleted` broadcast
//! back, and the spawned session actor emitting `SendToLlmProvider` — so a
//! fabric regression cannot silently reintroduce it.

use crate::orchestrator::{ToolOrchestratorActor, ToolOrchestratorActorDeps};
use jinn_core_types::tool_types::ToolCall;
use jinn_domain::common::bus::test_harness::{TestHarness, await_recorded};
use jinn_domain::common::state::State;
use jinn_domain::common::tcaps::mint;
use jinn_inference_msg::SendToLlmProvider;
use jinn_tools_msg::{ExecuteToolBatch, ToolBatchCompleted};
use std::time::Duration;

/// The orchestrator executes a real builtin (`bash`) dispatched over the bus
/// and emits `ToolBatchCompleted` with the tool's output.
#[rstest::rstest]
#[tokio::test]
async fn orchestrator_completes_builtin_bash_batch_dispatched_over_the_bus() {
    // Given a spawned orchestrator with the bash builtin registered.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<ToolBatchCompleted>().await;
    ToolOrchestratorActor::spawn(
        harness.system(),
        ToolOrchestratorActorDeps {
            deps: harness.actor_deps().await,
            state: State::new(jinn_domain::AppState::default()),
            session_cap: mint::mint_session_cap(),
            services: harness.services().await,
            builtin_filter: Some(vec!["bash".to_owned()]),
        },
    );

    let session_id = jinn_core_types::SessionId::new();

    // When an ExecuteToolBatch carrying one bash call is published on the bus.
    harness
        .publish(ExecuteToolBatch {
            session_id: session_id.clone(),
            tool_calls: vec![ToolCall {
                id: "tc-bash-1".to_owned(),
                name: "bash".to_owned(),
                arguments: r#"{"command":"echo fabric-loop-ok"}"#.to_owned(),
            }],
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;
    let batches = await_recorded::<ToolBatchCompleted>(&recorder, 1, Duration::from_secs(10)).await;

    // Then exactly one batch completed, for this session.
    let batches: Vec<_> = batches
        .into_iter()
        .filter(|b| b.session_id == session_id)
        .collect();
    assert_eq!(
        batches.len(),
        1,
        "expected exactly one ToolBatchCompleted for the session"
    );

    // And it carries the bash tool's successful result.
    let results = &batches[0].results;
    assert_eq!(results.len(), 1, "expected one tool result in the batch");
    let result = &results[0];
    assert_eq!(result.name, "bash");
    assert!(result.success, "bash tool call should succeed");
    assert!(
        result.content.contains("fabric-loop-ok"),
        "result should carry the command's stdout, got: {:?}",
        result.content
    );
}

/// The full tool loop closes over the fabric: `ToolBatchCompleted` published
/// by the orchestrator reaches the spawned session actor, which emits
/// `SendToLlmProvider` to continue the conversation.
#[rstest::rstest]
#[tokio::test]
async fn tool_batch_completed_over_the_bus_continues_the_tool_loop() {
    // Given a spawned orchestrator (bash) and a spawned session actor with a
    // tool-call entry in its history, phase Sending.
    let harness = TestHarness::new().await;
    let loop_recorder = harness.spawn_recorder::<SendToLlmProvider>().await;

    let state = State::new(jinn_domain::AppState::default());
    {
        let mut s = state.write_test_no_cap();
        let session = s.active_session_mut();
        session.push_entry(jinn_core_types::ChatEntry::user("list files"));
        session.push_entry(jinn_core_types::ChatEntry::assistant("checking"));
        session.push_entry(jinn_core_types::ChatEntry::tool_call(
            "tc-bash-2",
            "bash",
            r#"{"command":"echo loop-continues"}"#,
        ));
        session.begin_sending();
    }
    let session_id = state.read().session.active_session_id().clone();

    ToolOrchestratorActor::spawn(
        harness.system(),
        ToolOrchestratorActorDeps {
            deps: harness.actor_deps().await,
            state: state.clone(),
            session_cap: mint::mint_session_cap(),
            services: harness.services().await,
            builtin_filter: Some(vec!["bash".to_owned()]),
        },
    );
    jinn_session_turn::activate(
        harness.system(),
        jinn_session_turn::session_actor::SessionPersistenceActorDeps {
            deps: {
                let deps = harness.actor_deps().await;
                let _ =
                    jinn_context_assembly::service::ensure_spawned(&deps.services.trouper_system);
                deps
            },
            state,
            cap: mint::mint_session_cap(),
            frontend_cap: mint::mint_frontend_cap(),
            counter:
                jinn_domain::feat::context::strategy::token_estimator::TiktokenCounter::o200k_base(),
            token_cache: jinn_token_count_msg::HistoryWorkerChatEntryTokenCache::default(),
            image_converter: jinn_domain::feat::image_convert::ImageConverterService::unavailable(),
        },
    );

    // When the batch is dispatched over the bus (the orchestrator executes the
    // builtin and publishes ToolBatchCompleted itself).
    harness
        .publish(ExecuteToolBatch {
            session_id: session_id.clone(),
            tool_calls: vec![ToolCall {
                id: "tc-bash-2".to_owned(),
                name: "bash".to_owned(),
                arguments: r#"{"command":"echo loop-continues"}"#.to_owned(),
            }],
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;
    let sent =
        await_recorded::<SendToLlmProvider>(&loop_recorder, 1, Duration::from_secs(10)).await;

    // Then the session actor continued the tool loop.
    assert!(
        sent.iter().any(|m| m.session_id == session_id),
        "expected SendToLlmProvider after the executed batch reached the session actor"
    );
}

/// An MCP-shaped tool (actor tool, session-scoped registration) routes to its
/// provider via `ExecuteTool`; the provider's `ToolExecutionCompleted`
/// completes the batch. The registration/routing half of the "MCP tools never
/// returning" fix.
#[rstest::rstest]
#[tokio::test]
async fn registered_session_scoped_actor_tool_completes_its_batch() {
    use jinn_domain::common::actor_deps::BusPublish;
    use jinn_tools_msg::{ExecuteTool, RegisterTools, ToolExecutionCompleted};
    use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
    use trouper::context::MsgCtx;
    use trouper::registry::RegistryError;

    // Given a stub provider actor that answers ExecuteTool with a completed
    // result (like an MCP server would), and a spawned orchestrator.
    struct StubProvider {
        bus: jinn_domain::common::services::bus_service::BusService,
        session_id: jinn_core_types::SessionId,
    }
    impl ServiceActor for StubProvider {
        async fn start(
            _args: &trouper::json::Json,
        ) -> Result<Self, error_stack::Report<RegistryError>> {
            Err(error_stack::Report::new(RegistryError::InvalidSpec)
                .attach("StubProvider is spawned via start_with"))
        }
    }
    impl BusPublish for StubProvider {
        fn bus(&self) -> &jinn_domain::common::services::bus_service::BusService {
            &self.bus
        }
    }
    impl MsgHandler<ExecuteTool> for StubProvider {
        async fn handle(&mut self, msg: &ExecuteTool, _ctx: &mut MsgCtx<'_>) {
            self.publish(ToolExecutionCompleted {
                session_id: self.session_id.clone(),
                result: jinn_core_types::tool_types::ToolResult {
                    tool_call_id: msg.tool_call.id.clone(),
                    name: msg.tool_call.name.clone(),
                    content: "mcp-stub-answer".to_owned(),
                    success: true,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                },
            })
            .await;
        }
    }

    let harness = TestHarness::new().await;
    let batch_recorder = harness.spawn_recorder::<ToolBatchCompleted>().await;
    let session_id = jinn_core_types::SessionId::new();

    let _ = trouper::builder::spawn_service_builder::<StubProvider>(harness.system())
        .at(ActorPath::new("test.stub-mcp-provider"))
        .start_with({
            let bus = harness.bus();
            let session_id = session_id.clone();
            move || {
                let bus = bus.clone();
                let session_id = session_id.clone();
                Box::pin(async move { Ok(StubProvider { bus, session_id }) })
            }
        })
        .handles::<ExecuteTool>()
        .start();

    ToolOrchestratorActor::spawn(
        harness.system(),
        ToolOrchestratorActorDeps {
            deps: harness.actor_deps().await,
            state: State::new(jinn_domain::AppState::default()),
            session_cap: mint::mint_session_cap(),
            services: harness.services().await,
            builtin_filter: Some(vec![]),
        },
    );

    // When the provider registers an actor tool and a batch referencing it is
    // dispatched.
    harness
        .publish(RegisterTools {
            provider: "mcp__stub__".to_owned(),
            definitions: vec![jinn_core_types::tool_types::ToolDefinition {
                name: "mcp__stub__echo".to_owned(),
                description: "echo".to_owned(),
                parameters: serde_json::json!({"type": "object"}),
                prompt_snippet: None,
                prompt_guidelines: vec![],
                server_tool_type: None,
            }],
            session_id: Some(session_id.clone()),
        })
        .await;
    harness
        .publish(ExecuteToolBatch {
            session_id: session_id.clone(),
            tool_calls: vec![ToolCall {
                id: "tc-mcp-1".to_owned(),
                name: "mcp__stub__echo".to_owned(),
                arguments: r#"{"x":1}"#.to_owned(),
            }],
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;
    let batches =
        await_recorded::<ToolBatchCompleted>(&batch_recorder, 1, Duration::from_secs(10)).await;

    // Then the batch completed with the provider's answer.
    let batches: Vec<_> = batches
        .into_iter()
        .filter(|b| b.session_id == session_id)
        .collect();
    assert_eq!(
        batches.len(),
        1,
        "expected the MCP-shaped batch to complete"
    );
    let result = &batches[0].results[0];
    assert_eq!(result.name, "mcp__stub__echo");
    assert!(result.success);
    assert_eq!(result.content, "mcp-stub-answer");
}
