//! Contract tests for Anthropic provider using mockito.
//!
//! Verifies streaming chat, tool calls, auth headers, and response parsing.

use futures::StreamExt as _;
use nullslop_provider::{
    AnthropicFactory, LlmMessage, LlmService, LlmServiceFactory, StreamEvent, ToolDefinition,
};

fn make_service(server: &mockito::ServerGuard) -> nullslop_provider::anthropic::AnthropicService {
    nullslop_provider::anthropic::AnthropicService::with_base_url(
        "claude-3".into(),
        "test-key".into(),
        None,
        format!("{}/v1/messages", server.url()),
    )
}

#[tokio::test]
async fn text_streaming_yields_text_events() {
    let mut server = mockito::Server::new_async().await;
    let service = make_service(&server);

    let body = [
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
    ].join("");

    server
        .mock("POST", "/v1/messages")
        .match_header("x-api-key", "test-key")
        .match_header("anthropic-version", "2023-06-01")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(&body)
        .create_async()
        .await;

    let stream = service
        .chat_stream_with_tools(
            vec![LlmMessage::User {
                content: "hi".into(),
            }],
            vec![],
        )
        .await
        .unwrap();

    let events: Vec<StreamEvent> = stream.filter_map(|r| async move { r.ok() }).collect().await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::Text(t) if t == "Hello"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::Text(t) if t == " world"))
    );
    assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
}

#[tokio::test]
async fn tool_call_streaming_yields_start_delta_complete_done() {
    let mut server = mockito::Server::new_async().await;
    let service = make_service(&server);

    let body = [
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"echo\",\"input\":{}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"x\\\":1}\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
    ].join("");

    server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(&body)
        .create_async()
        .await;

    let stream = service
        .chat_stream_with_tools(
            vec![LlmMessage::User {
                content: "call echo".into(),
            }],
            vec![ToolDefinition {
                name: "echo".into(),
                description: "Echo".into(),
                parameters: serde_json::json!({"type": "object"}),
            }],
        )
        .await
        .unwrap();

    let events: Vec<StreamEvent> = stream.filter_map(|r| async move { r.ok() }).collect().await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolUseStart { name, .. } if name == "echo"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolUseInputDelta { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolUseComplete { .. }))
    );
    assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
}

#[test]
fn factory_rejects_empty_api_key() {
    let factory = AnthropicFactory::new("claude-3".into(), String::new(), "test".into());

    let result = factory.create();
    assert!(result.is_err());
}

#[tokio::test]
async fn chat_stream_yields_text_tokens_only() {
    let mut server = mockito::Server::new_async().await;
    let service = make_service(&server);

    let body = [
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
    ]
    .join("");

    server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(&body)
        .create_async()
        .await;

    let stream = service
        .chat_stream(vec![LlmMessage::User {
            content: "hi".into(),
        }])
        .await
        .unwrap();

    let tokens: Vec<String> = stream.filter_map(|r| async move { r.ok() }).collect().await;

    assert_eq!(tokens, vec!["Hello", " world"]);
}

#[tokio::test]
async fn error_response_401_returns_error() {
    let mut server = mockito::Server::new_async().await;
    let service = make_service(&server);

    server
        .mock("POST", "/v1/messages")
        .with_status(401)
        .with_body("{\"error\":\"unauthorized\"}")
        .create_async()
        .await;

    let result = service
        .chat_stream_with_tools(
            vec![LlmMessage::User {
                content: "hi".into(),
            }],
            vec![],
        )
        .await;

    assert!(result.is_err());
}
