#![allow(clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]

use std::path::PathBuf;

use crate::common::actor::{Actor, ActorContext, RecordingSink};
use crate::common::app_state::AppState;
use crate::common::state::State;
use crate::feat::tools_actor::get_time;
use crate::feat::tools_actor::protocol::command::{
    CancelToolBatch, ExecuteToolBatch, RegisterTools,
};
use crate::feat::tools_actor::protocol::event::{ToolBatchCompleted, ToolExecutionCompleted};
use crate::feat::tools_actor::read;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::feat::tools_actor::write;
use crate::protocol::{Command, Event, SessionId};

use super::{ToolOrchestratorActor, ToolOrchestratorActorDeps, ToolRegistration};

/// Creates a test context backed by a recording sink.
fn _test_context(sink: &std::sync::Arc<RecordingSink>) -> ActorContext {
    ActorContext::new("test-tool-orchestrator", sink.clone())
}

/// Creates a test context with State injection.
fn test_context_with_state(sink: &std::sync::Arc<RecordingSink>, _state: State) -> ActorContext {
    ActorContext::new("test-tool-orchestrator", sink.clone())
}

fn default_test_ctx() -> (
    std::sync::Arc<RecordingSink>,
    ActorContext,
    ToolOrchestratorActorDeps,
) {
    let sink = std::sync::Arc::new(RecordingSink::new());
    let state = State::new(AppState::default());
    let ctx = test_context_with_state(&sink, state.clone());
    let test_services = crate::common::services::test_services::TestServices::builder().build();
    let deps = ToolOrchestratorActorDeps {
        services: test_services,
        state,
        builtin_filter: None,
        shell: "/bin/sh".to_owned(),
    };
    (sink, ctx, deps)
}

/// Extracts `ToolBatchCompleted` events from a list of events.
fn find_batch_completed(events: &[Event]) -> Vec<&ToolBatchCompleted> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ToolBatchCompleted(payload) => Some(payload),
            _ => None,
        })
        .collect()
}

/// Extracts `ToolExecutionCompleted` events from a list of events.
fn find_execution_completed(events: &[Event]) -> Vec<&ToolExecutionCompleted> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ToolExecutionCompleted(payload) => Some(payload),
            _ => None,
        })
        .collect()
}

// --- Activation tests ---

#[rstest::rstest]
#[tokio::test]
async fn activate_registers_get_time_tool() {
    // Given a fresh actor context with state.
    let (_sink, mut ctx, deps) = default_test_ctx();

    // When activating the actor.
    let actor = ToolOrchestratorActor::activate(deps, &mut ctx);

    // Then the get_time tool is registered.
    assert!(actor.tools.contains_key("get_time"));
}

#[rstest::rstest]
#[tokio::test]
async fn activate_registers_read_tool() {
    // Given a fresh actor context with state.
    let (_sink, mut ctx, deps) = default_test_ctx();

    // When activating the actor.
    let actor = ToolOrchestratorActor::activate(deps, &mut ctx);

    // Then the read tool is registered.
    assert!(actor.tools.contains_key("read"));
}

#[rstest::rstest]
#[tokio::test]
async fn activate_registers_write_tool() {
    // Given a fresh actor context with state.
    let (_sink, mut ctx, deps) = default_test_ctx();

    // When activating the actor.
    let actor = ToolOrchestratorActor::activate(deps, &mut ctx);

    // Then the write tool is registered.
    assert!(actor.tools.contains_key("write"));
}

#[rstest::rstest]
#[tokio::test]
async fn activate_emits_tools_registered_for_builtins() {
    // Given a fresh actor context with state and a recording sink.
    let (sink, mut ctx, deps) = default_test_ctx();

    // When activating the actor.
    let _actor = ToolOrchestratorActor::activate(deps, &mut ctx);

    // Then ToolsRegistered event was emitted for built-in tools.
    let events = sink.events();
    let tools_registered: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::ToolsRegistered(payload) => Some(payload.clone()),
            _ => None,
        })
        .collect();

    let builtin_evt = tools_registered
        .iter()
        .find(|p| p.provider == "builtin")
        .expect("expected builtin ToolsRegistered");
    assert_eq!(builtin_evt.definitions.len(), 16);
}

// --- RegisterTools command tests ---

#[rstest::rstest]
#[tokio::test]
async fn register_tools_stores_actor_tools() {
    // Given an activated actor.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let definition = ToolDefinition {
        name: "web_search".to_owned(),
        description: "Search the web".to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        server_tool_type: None,
    };

    // When registering an actor-provided tool.
    let cmd = Command::RegisterTools(RegisterTools {
        provider: "web-actor".to_owned(),
        definitions: vec![definition],
    });
    actor.handle_command(&cmd, &ctx);

    // Then the tool is stored in the registry.
    let reg = actor
        .get_tool("web_search")
        .expect("tool should be registered");
    match reg {
        ToolRegistration::Actor { provider, .. } => {
            assert_eq!(provider, "web-actor");
        }
        other @ ToolRegistration::Builtin { .. } => {
            panic!("expected Actor registration, got {other:?}")
        }
    }
}

#[rstest::rstest]
#[tokio::test]
async fn register_tools_emits_event() {
    // Given an activated actor.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let definition = ToolDefinition {
        name: "web_search".to_owned(),
        description: "Search the web".to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        server_tool_type: None,
    };

    // When registering tools.
    let cmd = Command::RegisterTools(RegisterTools {
        provider: "web-actor".to_owned(),
        definitions: vec![definition.clone()],
    });
    actor.handle_command(&cmd, &ctx);

    // Then a ToolsRegistered event is emitted.
    let events = sink.events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::ToolsRegistered(payload) => {
            assert_eq!(payload.provider, "web-actor");
        }
        other => panic!("expected ToolsRegistered, got {other:?}"),
    }
}

#[rstest::rstest]
#[tokio::test]
async fn register_tools_records_tool_count() {
    // Given an activated actor.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let definition = ToolDefinition {
        name: "web_search".to_owned(),
        description: "Search the web".to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        server_tool_type: None,
    };

    // When registering tools.
    let cmd = Command::RegisterTools(RegisterTools {
        provider: "web-actor".to_owned(),
        definitions: vec![definition.clone()],
    });
    actor.handle_command(&cmd, &ctx);

    // Then the event contains the correct definitions.
    let events = sink.events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::ToolsRegistered(payload) => {
            assert_eq!(payload.definitions.len(), 1);
            assert_eq!(payload.definitions[0].name, "web_search");
        }
        other => panic!("expected ToolsRegistered, got {other:?}"),
    }
}

// --- Built-in tool execution tests ---

#[rstest::rstest]
#[tokio::test]
async fn execute_builtin_get_time_tool() {
    // Given a get_time tool call.
    let call = ToolCall {
        id: "call_3".to_owned(),
        name: "get_time".to_owned(),
        arguments: "{}".to_owned(),
    };
    let ctx = ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        bash_default_timeout: None,
        state: None,
        session_id: None,
        app_paths: crate::common::app_paths::AppPaths::default(),
        sink: None,
        shell: "/bin/sh".to_owned(),
        max_output_lines: None,
        max_output_bytes: None,
    };

    // When executing the get_time tool.
    let result = get_time::execute(call, ctx).await;

    // Then the result has non-empty content.
    assert_eq!(result.tool_call_id, "call_3");
    assert!(result.success);
    assert!(!result.content.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn execute_builtin_read_tool() {
    // Given a temp file with known content.
    let dir = tempfile::tempdir().expect("create temp dir");
    let file_path = dir.path().join("test.txt");
    std::fs::write(&file_path, "file contents here").expect("write temp file");

    let call = ToolCall {
        id: "call_4".to_owned(),
        name: "read".to_owned(),
        arguments: serde_json::json!({
            "path": file_path.to_string_lossy()
        })
        .to_string(),
    };
    let tool_ctx = ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        bash_default_timeout: None,
        state: None,
        session_id: None,
        app_paths: crate::common::app_paths::AppPaths::default(),
        sink: None,
        shell: "/bin/sh".to_owned(),
        max_output_lines: None,
        max_output_bytes: None,
    };

    // When executing the read tool.
    let result = read::execute(call, tool_ctx).await;
    assert_eq!(result.tool_call_id, "call_4");
    assert!(result.success);
    assert!(result.content.contains("file contents here"));
    assert!(
        result.content.contains('#'),
        "expected LINE#HASH annotation"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn execute_builtin_read_tool_returns_error_on_missing_file() {
    // Given a read call for a nonexistent file.
    let call = ToolCall {
        id: "call_5".to_owned(),
        name: "read".to_owned(),
        arguments: serde_json::json!({
            "path": "/nonexistent/path/to/file.txt"
        })
        .to_string(),
    };
    let tool_ctx = ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        bash_default_timeout: None,
        state: None,
        session_id: None,
        app_paths: crate::common::app_paths::AppPaths::default(),
        sink: None,
        shell: "/bin/sh".to_owned(),
        max_output_lines: None,
        max_output_bytes: None,
    };

    // When executing the read tool.
    let result = read::execute(call, tool_ctx).await;

    // Then the result indicates failure.
    assert!(!result.success);
    assert!(result.content.contains("failed to read file"));
}

// --- Batch execution tests ---

#[rstest::rstest]
#[tokio::test]
async fn execute_batch_with_get_time_tool_emits_completion() {
    // Given an activated actor.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    // When executing a batch with one get_time call.
    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![ToolCall {
            id: "call_1".to_owned(),
            name: "get_time".to_owned(),
            arguments: "{}".to_owned(),
        }],
    });
    actor.handle_command(&cmd, &ctx);

    // Then a ToolExecutionCompleted event arrives from the spawned task.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = sink.take_events();
    let completed = find_execution_completed(&events);
    assert_eq!(completed.len(), 1);
    assert!(completed[0].result.success);
    assert!(!completed[0].result.content.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn completion_event_triggers_batch_completed() {
    // Given an activated actor with a single get_time batch executed.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![ToolCall {
            id: "call_1".to_owned(),
            name: "get_time".to_owned(),
            arguments: "{}".to_owned(),
        }],
    });
    actor.handle_command(&cmd, &ctx);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = sink.take_events();
    let completed = find_execution_completed(&events);

    // When feeding the completion event back to the actor.
    let completion_event = Event::ToolExecutionCompleted(ToolExecutionCompleted {
        session_id: session_id.clone(),
        result: completed[0].result.clone(),
    });
    actor.handle_event(&completion_event, &ctx);

    // Then a ToolBatchCompleted event is emitted.
    let events = sink.events();
    let batch_completed = find_batch_completed(&events);
    assert_eq!(batch_completed.len(), 1);
    assert_eq!(batch_completed[0].results.len(), 1);
    assert!(!batch_completed[0].results[0].content.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn execute_batch_with_two_tools_emits_two_completions() {
    // Given an activated actor.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    // When executing a batch with two get_time calls.
    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![
            ToolCall {
                id: "call_a".to_owned(),
                name: "get_time".to_owned(),
                arguments: "{}".to_owned(),
            },
            ToolCall {
                id: "call_b".to_owned(),
                name: "get_time".to_owned(),
                arguments: "{}".to_owned(),
            },
        ],
    });
    actor.handle_command(&cmd, &ctx);

    // Then two ToolExecutionCompleted events arrive.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = sink.take_events();
    let completed = find_execution_completed(&events);
    assert_eq!(completed.len(), 2);
}

#[rstest::rstest]
#[tokio::test]
async fn first_completion_does_not_complete_batch() {
    // Given an activated actor with a batch of two get_time calls executed.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![
            ToolCall {
                id: "call_a".to_owned(),
                name: "get_time".to_owned(),
                arguments: "{}".to_owned(),
            },
            ToolCall {
                id: "call_b".to_owned(),
                name: "get_time".to_owned(),
                arguments: "{}".to_owned(),
            },
        ],
    });
    actor.handle_command(&cmd, &ctx);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = sink.take_events();
    let completed = find_execution_completed(&events);

    // When feeding the first completion back.
    actor.handle_event(
        &Event::ToolExecutionCompleted(ToolExecutionCompleted {
            session_id: session_id.clone(),
            result: completed[0].result.clone(),
        }),
        &ctx,
    );

    // Then no batch completed yet (one remaining).
    let events = sink.take_events();
    assert!(find_batch_completed(&events).is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn second_completion_emits_batch_completed() {
    // Given an activated actor with a batch of two get_time calls where first completion was fed back.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![
            ToolCall {
                id: "call_a".to_owned(),
                name: "get_time".to_owned(),
                arguments: "{}".to_owned(),
            },
            ToolCall {
                id: "call_b".to_owned(),
                name: "get_time".to_owned(),
                arguments: "{}".to_owned(),
            },
        ],
    });
    actor.handle_command(&cmd, &ctx);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = sink.take_events();
    let completed = find_execution_completed(&events);

    actor.handle_event(
        &Event::ToolExecutionCompleted(ToolExecutionCompleted {
            session_id: session_id.clone(),
            result: completed[0].result.clone(),
        }),
        &ctx,
    );
    sink.take_events();

    // When feeding the second completion back.
    actor.handle_event(
        &Event::ToolExecutionCompleted(ToolExecutionCompleted {
            session_id: session_id.clone(),
            result: completed[1].result.clone(),
        }),
        &ctx,
    );

    // Then ToolBatchCompleted is emitted with both results.
    let events = sink.events();
    let batch_completed = find_batch_completed(&events);
    assert_eq!(batch_completed.len(), 1);
    assert_eq!(batch_completed[0].results.len(), 2);
}

#[rstest::rstest]
#[tokio::test]
async fn execute_batch_with_unknown_tool_emits_error_completion() {
    // Given an activated actor.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    // When executing a batch with an unknown tool name.
    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![ToolCall {
            id: "call_x".to_owned(),
            name: "nonexistent_tool".to_owned(),
            arguments: "{}".to_owned(),
        }],
    });
    actor.handle_command(&cmd, &ctx);

    // Then a ToolExecutionCompleted event with an error is emitted synchronously.
    let events = sink.events();
    let completed = find_execution_completed(&events);
    assert_eq!(completed.len(), 1);
    assert!(!completed[0].result.success);
    assert!(completed[0].result.content.contains("unknown tool"));
}

#[rstest::rstest]
#[tokio::test]
async fn error_completion_triggers_batch_completed() {
    // Given an activated actor with an unknown tool batch executed.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![ToolCall {
            id: "call_x".to_owned(),
            name: "nonexistent_tool".to_owned(),
            arguments: "{}".to_owned(),
        }],
    });
    actor.handle_command(&cmd, &ctx);

    let events = sink.events();
    let completed = find_execution_completed(&events);

    // When feeding the error result back.
    actor.handle_event(
        &Event::ToolExecutionCompleted(ToolExecutionCompleted {
            session_id: session_id.clone(),
            result: completed[0].result.clone(),
        }),
        &ctx,
    );

    // Then ToolBatchCompleted is emitted with the error result.
    let events = sink.events();
    let batch_completed = find_batch_completed(&events);
    assert_eq!(batch_completed.len(), 1);
    assert_eq!(batch_completed[0].results.len(), 1);
    assert!(!batch_completed[0].results[0].success);
}

#[rstest::rstest]
#[tokio::test]
async fn execute_batch_with_no_tool_calls_emits_empty_batch_completed() {
    // Given an activated actor.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    // When executing a batch with no tool calls.
    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![],
    });
    actor.handle_command(&cmd, &ctx);

    // Then an empty ToolBatchCompleted is emitted immediately.
    let events = sink.events();
    let batch_completed = find_batch_completed(&events);
    assert_eq!(batch_completed.len(), 1);
    assert!(batch_completed[0].results.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn write_tool_returns_success() {
    // Given a temp directory.
    let dir = tempfile::tempdir().expect("create temp dir");
    let file_path = dir.path().join("output.txt");

    let call = ToolCall {
        id: "call_w1".to_owned(),
        name: "write".to_owned(),
        arguments: serde_json::json!({
            "path": file_path.to_string_lossy(),
            "content": "hello from write"
        })
        .to_string(),
    };
    let tool_ctx = ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        bash_default_timeout: None,
        state: None,
        session_id: None,
        app_paths: crate::common::app_paths::AppPaths::default(),
        sink: None,
        shell: "/bin/sh".to_owned(),
        max_output_lines: None,
        max_output_bytes: None,
    };

    // When executing the write tool.
    let result = write::execute(call, tool_ctx).await;

    // Then the result indicates success.
    assert_eq!(result.tool_call_id, "call_w1");
    assert!(result.success, "expected success, got: {}", result.content);
    assert!(result.content.contains("wrote 16 bytes"));
}

#[rstest::rstest]
#[tokio::test]
async fn write_tool_creates_file_with_content() {
    // Given a temp directory.
    let dir = tempfile::tempdir().expect("create temp dir");
    let file_path = dir.path().join("output.txt");

    let call = ToolCall {
        id: "call_w1".to_owned(),
        name: "write".to_owned(),
        arguments: serde_json::json!({
            "path": file_path.to_string_lossy(),
            "content": "hello from write"
        })
        .to_string(),
    };
    let tool_ctx = ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        bash_default_timeout: None,
        state: None,
        session_id: None,
        app_paths: crate::common::app_paths::AppPaths::default(),
        sink: None,
        shell: "/bin/sh".to_owned(),
        max_output_lines: None,
        max_output_bytes: None,
    };

    // When executing the write tool.
    let _result = write::execute(call, tool_ctx).await;

    // Then the file contains the written content.
    let content = std::fs::read_to_string(&file_path).expect("read written file");
    assert_eq!(content, "hello from write");
}

#[rstest::rstest]
#[tokio::test]
async fn write_tool_creates_parent_dirs_and_file() {
    // Given a temp directory with a nested path.
    let dir = tempfile::tempdir().expect("create temp dir");
    let file_path = dir.path().join("nested").join("deep").join("file.txt");

    let call = ToolCall {
        id: "call_w2".to_owned(),
        name: "write".to_owned(),
        arguments: serde_json::json!({
            "path": file_path.to_string_lossy(),
            "content": "nested content"
        })
        .to_string(),
    };
    let tool_ctx = ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        bash_default_timeout: None,
        state: None,
        session_id: None,
        app_paths: crate::common::app_paths::AppPaths::default(),
        sink: None,
        shell: "/bin/sh".to_owned(),
        max_output_lines: None,
        max_output_bytes: None,
    };

    // When executing the write tool.
    let result = write::execute(call, tool_ctx).await;

    // Then the result indicates success.
    assert!(result.success, "expected success, got: {}", result.content);

    // And the file was created with parent directories.
    let content = std::fs::read_to_string(&file_path).expect("read written file");
    assert_eq!(content, "nested content");
}

#[rstest::rstest]
#[tokio::test]
async fn write_tool_overwrites_existing_file() {
    // Given a temp file with existing content.
    let dir = tempfile::tempdir().expect("create temp dir");
    let file_path = dir.path().join("existing.txt");
    std::fs::write(&file_path, "old content").expect("write existing file");

    let call = ToolCall {
        id: "call_w3".to_owned(),
        name: "write".to_owned(),
        arguments: serde_json::json!({
            "path": file_path.to_string_lossy(),
            "content": "new content"
        })
        .to_string(),
    };
    let tool_ctx = ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        bash_default_timeout: None,
        state: None,
        session_id: None,
        app_paths: crate::common::app_paths::AppPaths::default(),
        sink: None,
        shell: "/bin/sh".to_owned(),
        max_output_lines: None,
        max_output_bytes: None,
    };

    // When executing the write tool.
    let result = write::execute(call, tool_ctx).await;

    // Then the result indicates success.
    assert!(result.success);
    let content = std::fs::read_to_string(&file_path).expect("read overwritten file");
    assert_eq!(content, "new content");
}

#[rstest::rstest]
#[tokio::test]
async fn write_tool_returns_error_on_bad_json() {
    // Given a write call with invalid JSON.
    let call = ToolCall {
        id: "call_w4".to_owned(),
        name: "write".to_owned(),
        arguments: "not json".to_owned(),
    };
    let tool_ctx = ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        bash_default_timeout: None,
        state: None,
        session_id: None,
        app_paths: crate::common::app_paths::AppPaths::default(),
        sink: None,
        shell: "/bin/sh".to_owned(),
        max_output_lines: None,
        max_output_bytes: None,
    };

    // When executing the write tool.
    let result = write::execute(call, tool_ctx).await;

    // Then the result indicates failure.
    assert_eq!(result.tool_call_id, "call_w4");
    assert!(!result.success);
    assert!(result.content.contains("failed to parse arguments"));
}

#[rstest::rstest]
#[tokio::test]
async fn tool_execution_completed_for_unknown_session_is_ignored() {
    // Given an activated actor with no pending batches.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let unknown_session = SessionId::new();

    // When receiving a ToolExecutionCompleted for an unknown session.
    let event = Event::ToolExecutionCompleted(ToolExecutionCompleted {
        session_id: unknown_session,
        result: ToolResult {
            tool_call_id: "call_0".to_owned(),
            name: "get_time".to_owned(),
            content: "orphan".to_owned(),
            success: true,
            full_content: None,
            truncation: None,
            pin_position: None,
        },
    });
    actor.handle_event(&event, &ctx);

    // Then no batch completed event is emitted.
    let events = sink.events();
    let batch_completed = find_batch_completed(&events);
    assert!(batch_completed.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn build_tool_context_reads_session_cwd() {
    // Given an activated actor with a session that has a specific CWD.
    let (sink, mut ctx, deps) = default_test_ctx();
    let actor = ToolOrchestratorActor::activate(deps, &mut ctx);

    let session_id = {
        let mut guard = actor.state.write();
        let session = guard.active_session_mut();
        session.set_cwd(PathBuf::from("/custom/cwd"));
        guard.session.active_session_id().clone()
    };

    // When building tool context for that session.
    let tool_ctx = actor.build_tool_context(&session_id, sink.clone());

    // Then the CWD matches the session's CWD.
    assert_eq!(tool_ctx.cwd, PathBuf::from("/custom/cwd"));
}

#[rstest::rstest]
#[tokio::test]
async fn build_tool_context_returns_default_cwd_for_unknown_session() {
    // Given an activated actor.
    let (sink, mut ctx, deps) = default_test_ctx();
    let actor = ToolOrchestratorActor::activate(deps, &mut ctx);

    // When building tool context for an unknown session.
    let unknown_session = SessionId::new();
    let tool_ctx = actor.build_tool_context(&unknown_session, sink.clone());

    // Then the CWD falls back to default_cwd (which is "/" by default).
    assert_eq!(tool_ctx.cwd, PathBuf::from("/"));
}

// --- CancelToolBatch tests ---

#[rstest::rstest]
#[tokio::test]
async fn cancel_tool_batch_removes_pending_batch() {
    // Given an activated actor with a pending batch of two get_time calls.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![
            ToolCall {
                id: "call_a".to_owned(),
                name: "get_time".to_owned(),
                arguments: "{}".to_owned(),
            },
            ToolCall {
                id: "call_b".to_owned(),
                name: "get_time".to_owned(),
                arguments: "{}".to_owned(),
            },
        ],
    });
    actor.handle_command(&cmd, &ctx);

    // Then the pending batch exists.
    assert!(actor.pending.contains_key(&session_id));

    // When cancelling the tool batch.
    let cancel_cmd = Command::CancelToolBatch(CancelToolBatch {
        session_id: session_id.clone(),
    });
    actor.handle_command(&cancel_cmd, &ctx);

    // Then the pending batch is removed.
    assert!(!actor.pending.contains_key(&session_id));
}

#[rstest::rstest]
#[tokio::test]
async fn cancel_tool_batch_aborts_spawned_tasks() {
    // Given an activated actor with a pending batch of get_time calls.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let session_id = SessionId::new();

    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![ToolCall {
            id: "call_1".to_owned(),
            name: "get_time".to_owned(),
            arguments: "{}".to_owned(),
        }],
    });
    actor.handle_command(&cmd, &ctx);

    // Verify the batch has a spawned handle.
    let handle_count = actor
        .pending
        .get(&session_id)
        .map_or(0, |b| b.handles.len());
    assert_eq!(handle_count, 1, "should have one spawned task handle");

    // When cancelling the tool batch.
    let cancel_cmd = Command::CancelToolBatch(CancelToolBatch {
        session_id: session_id.clone(),
    });
    actor.handle_command(&cancel_cmd, &ctx);

    // Then the pending batch is removed (handles were aborted).
    assert!(!actor.pending.contains_key(&session_id));
    // And no ToolBatchCompleted was emitted (cancellation doesn't emit batch complete).
    let events = sink.events();
    let batch_completed = find_batch_completed(&events);
    assert!(batch_completed.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn cancel_tool_batch_for_unknown_session_is_noop() {
    // Given an activated actor with no pending batches.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let unknown_session = SessionId::new();

    // When cancelling a tool batch for an unknown session.
    let cancel_cmd = Command::CancelToolBatch(CancelToolBatch {
        session_id: unknown_session,
    });
    actor.handle_command(&cancel_cmd, &ctx);

    // Then no events are emitted and no panic.
    let events = sink.events();
    let batch_completed = find_batch_completed(&events);
    assert!(batch_completed.is_empty());
}

// --- build_tool_context CWD fallback tests ---

#[rstest::rstest]
#[tokio::test]
async fn build_tool_context_uses_session_default_cwd_when_not_overridden() {
    // Given an activated actor with a session using the default CWD (".").
    let (sink, mut ctx, deps) = default_test_ctx();
    let actor = ToolOrchestratorActor::activate(deps, &mut ctx);

    let session_id = {
        let mut guard = actor.state.write();
        let _session = guard.active_session_mut();
        // Don't set cwd - it defaults to ".".
        guard.session.active_session_id().clone()
    };

    // When building tool context for that session.
    let tool_ctx = actor.build_tool_context(&session_id, sink.clone());

    // Then the session cwd is used ("." from default).
    assert_eq!(tool_ctx.cwd, PathBuf::from("."));
}

#[rstest::rstest]
#[tokio::test]
async fn handle_processes_register_tools_command() {
    // Given an activated actor.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    let definition = ToolDefinition {
        name: "test_tool".to_owned(),
        description: "A test tool".to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({"type": "object", "properties": {}}),
        server_tool_type: None,
    };

    // When calling handle with ActorEnvelope::Command(RegisterTools).
    let cmd = Command::RegisterTools(RegisterTools {
        provider: "test-provider".to_owned(),
        definitions: vec![definition],
    });
    actor
        .handle(crate::common::actor::ActorEnvelope::Command(cmd), &ctx)
        .await;

    // Then the tool was registered (event emitted).
    let events = sink.events();
    let found = events
        .iter()
        .any(|e| matches!(e, Event::ToolsRegistered(_)));
    assert!(found, "handle should process RegisterTools command");
}

#[rstest::rstest]
#[tokio::test]
async fn handle_processes_tool_execution_completed_event() {
    // Given an activated actor with a pending empty batch.
    let (sink, mut ctx, deps) = default_test_ctx();
    let mut actor = ToolOrchestratorActor::activate(deps, &mut ctx);
    sink.clear();

    // Execute a batch with an unknown tool (gets immediate error completion).
    let session_id = SessionId::new();
    let cmd = Command::ExecuteToolBatch(ExecuteToolBatch {
        session_id: session_id.clone(),
        tool_calls: vec![ToolCall {
            id: "call_err".to_owned(),
            name: "nonexistent".to_owned(),
            arguments: "{}".to_owned(),
        }],
    });
    actor.handle_command(&cmd, &ctx);
    sink.take_events(); // Clear the error completion event.

    // Feed the completion back via handle (Envelope::Event path).
    let _events = sink.events();
    let completed_evt = Event::ToolExecutionCompleted(ToolExecutionCompleted {
        session_id: session_id.clone(),
        result: ToolResult {
            tool_call_id: "call_err".to_owned(),
            name: "nonexistent".to_owned(),
            content: "unknown tool".to_owned(),
            success: false,
            full_content: None,
            truncation: None,
            pin_position: None,
        },
    });
    actor
        .handle(
            crate::common::actor::ActorEnvelope::Event(completed_evt),
            &ctx,
        )
        .await;

    // Then ToolBatchCompleted was emitted via the event path.
    let events = sink.events();
    let batch_completed = find_batch_completed(&events);
    assert_eq!(
        batch_completed.len(),
        1,
        "handle should process ToolExecutionCompleted event"
    );
}

#[rstest::rstest]
fn tool_registration_debug_shows_name() {
    // Given a Builtin tool registration.
    let reg = ToolRegistration::Builtin {
        definition: ToolDefinition {
            name: "my_tool".to_owned(),
            description: "test".to_owned(),
            prompt_snippet: None,
            prompt_guidelines: vec![],
            parameters: serde_json::json!({}),
            server_tool_type: None,
        },
        execute: |_call, _ctx| Box::pin(async { unimplemented!() }),
    };

    // When formatting as Debug.
    let debug_str = format!("{reg:?}");

    // Then the output contains the tool name.
    assert!(
        debug_str.contains("my_tool"),
        "Debug output should contain tool name: {debug_str}"
    );
}
