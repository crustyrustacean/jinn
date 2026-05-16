//! Contract tests for Google Gemini provider using mockito.

use futures::StreamExt as _;
use nullslop_provider::{
    GoogleFactory, LlmMessage, LlmService, LlmServiceFactory, StreamEvent, ToolDefinition,
};

fn make_service(server: &mockito::ServerGuard) -> nullslop_provider::google::GoogleService {
    nullslop_provider::google::GoogleService::with_base_url(
        "gemini-pro".into(),
        "test-key".into(),
        server.url(),
    )
}

#[tokio::test]
async fn text_streaming_yields_text_events() {
    let mut server = mockito::Server::new_async().await;
    let service = make_service(&server);

    let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]},\"finishReason\":\"STOP\"}]}\n\n";

    server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"\?key=test-key".to_owned()),
        )
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
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
}

#[test]
fn factory_rejects_empty_api_key() {
    let factory = GoogleFactory::new("gemini-pro".into(), String::new(), "test".into());

    let result = factory.create();
    assert!(result.is_err());
}

#[tokio::test]
async fn tool_call_streaming_yields_start_delta_complete_done() {
    let mut server = mockito::Server::new_async().await;
    let service = make_service(&server);

    let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"echo\",\"args\":{\"x\":1}}}]},\"finishReason\":\"STOP\"}]}\n\n";

    server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"\?key=test-key".to_owned()),
        )
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
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

#[tokio::test]
async fn error_response_401_returns_error() {
    let mut server = mockito::Server::new_async().await;
    let service = make_service(&server);

    server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"\?key=test-key".to_owned()),
        )
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

#[tokio::test]
async fn chat_stream_yields_text_tokens_only() {
    let mut server = mockito::Server::new_async().await;
    let service = make_service(&server);

    let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]},\"finishReason\":\"STOP\"}]}\n\n";

    server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"\?key=test-key".to_owned()),
        )
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .create_async()
        .await;

    let stream = service
        .chat_stream(vec![LlmMessage::User {
            content: "hi".into(),
        }])
        .await
        .unwrap();

    let tokens: Vec<String> = stream.filter_map(|r| async move { r.ok() }).collect().await;

    assert_eq!(tokens, vec!["Hello"]);
}
