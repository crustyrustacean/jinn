//! Contract tests for OpenAI-compatible provider using mockito.
//!
//! Verifies streaming chat, tool calls, model listing, extra_body,
//! base_url override, auth headers, and error handling.

use futures::StreamExt as _;
use nullslop_provider::{
    LlmMessage, LlmServiceFactory, OpenAiCompatibleFactory, ProviderConfig, StreamEvent,
    ToolDefinition,
};

fn make_factory(server: &mockito::ServerGuard) -> OpenAiCompatibleFactory {
    let config = ProviderConfig::openai();
    OpenAiCompatibleFactory::new(
        config,
        "gpt-4".to_owned(),
        Some(server.url()),
        "test-key".to_owned(),
        None,
        "test-openai".to_owned(),
    )
}

fn make_factory_with_extra(
    server: &mockito::ServerGuard,
    extra: serde_json::Value,
) -> OpenAiCompatibleFactory {
    let config = ProviderConfig::openai();
    OpenAiCompatibleFactory::new(
        config,
        "gpt-4".to_owned(),
        Some(server.url()),
        "test-key".to_owned(),
        Some(extra),
        "test-openai".to_owned(),
    )
}

#[tokio::test]
async fn text_streaming_yields_text_tokens_then_done() {
    let mut server = mockito::Server::new_async().await;
    let factory = make_factory(&server);

    let body = "data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";

    server
        .mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer test-key")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .create_async()
        .await;

    let service = factory.create().unwrap();
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
    let factory = make_factory(&server);

    let body = [
        "data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"echo\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"x\\\":1}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    ].join("");

    server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(&body)
        .create_async()
        .await;

    let service = factory.create().unwrap();
    let stream = service
        .chat_stream_with_tools(
            vec![LlmMessage::User {
                content: "call echo".into(),
            }],
            vec![ToolDefinition {
                name: "echo".into(),
                description: "Echo".into(),
                prompt_snippet: None,
                prompt_guidelines: vec![],
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
async fn list_models_returns_model_ids() {
    let mut server = mockito::Server::new_async().await;

    server
        .mock("GET", "/models")
        .match_header("authorization", "Bearer test-key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("{\"data\":[{\"id\":\"gpt-4\"},{\"id\":\"gpt-3.5-turbo\"}]}")
        .create_async()
        .await;

    let config = ProviderConfig::openai();
    let svc = nullslop_provider::OpenAiCompatibleService::new(
        config,
        "gpt-4".into(),
        Some(server.url()),
        "test-key".into(),
        None,
    );

    let models = svc.list_models().await.unwrap();
    assert!(models.contains(&"gpt-4".to_owned()));
    assert!(models.contains(&"gpt-3.5-turbo".to_owned()));
}

#[tokio::test]
async fn extra_body_fields_included_in_request() {
    let mut server = mockito::Server::new_async().await;
    let factory = make_factory_with_extra(
        &server,
        serde_json::json!({"enable_thinking": true, "tool_stream": true}),
    );

    let mock = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJsonString(
            "{\"enable_thinking\":true,\"tool_stream\":true}".to_owned(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body("data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n")
        .create_async()
        .await;

    let service = factory.create().unwrap();
    let stream = service
        .chat_stream_with_tools(
            vec![LlmMessage::User {
                content: "test".into(),
            }],
            vec![],
        )
        .await
        .unwrap();

    let _events: Vec<StreamEvent> = stream.filter_map(|r| async move { r.ok() }).collect().await;

    mock.assert_async().await;
}

#[tokio::test]
async fn error_response_401_mapped_to_provider_error() {
    let mut server = mockito::Server::new_async().await;
    let factory = make_factory(&server);

    server
        .mock("POST", "/chat/completions")
        .with_status(401)
        .with_body("{\"error\":\"unauthorized\"}")
        .create_async()
        .await;

    let service = factory.create().unwrap();
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
async fn reasoning_content_produces_reasoning_event() {
    let mut server = mockito::Server::new_async().await;
    let factory = make_factory(&server);

    let body = "data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"thinking...\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";

    server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .create_async()
        .await;

    let service = factory.create().unwrap();
    let stream = service
        .chat_stream_with_tools(
            vec![LlmMessage::User {
                content: "think".into(),
            }],
            vec![],
        )
        .await
        .unwrap();

    let events: Vec<StreamEvent> = stream.filter_map(|r| async move { r.ok() }).collect().await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::Reasoning(t) if t == "thinking..."))
    );
}

#[tokio::test]
async fn base_url_override_routes_to_correct_host() {
    let mut server = mockito::Server::new_async().await;
    let config = ProviderConfig::openrouter();
    let factory = OpenAiCompatibleFactory::new(
        config,
        "moonshotai/kimi-k2:free".to_owned(),
        Some(server.url()),
        "test-key".to_owned(),
        None,
        "test-openrouter".to_owned(),
    );

    server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body("data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n")
        .create_async()
        .await;

    let service = factory.create().unwrap();
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

    assert!(!events.is_empty());
}

#[test]
fn factory_rejects_empty_api_key() {
    let config = ProviderConfig::openai();
    let factory = OpenAiCompatibleFactory::new(
        config,
        "gpt-4".to_owned(),
        None,
        String::new(),
        None,
        "test-openai".to_owned(),
    );

    let result = factory.create();
    assert!(result.is_err());
}

#[tokio::test]
async fn chat_stream_yields_text_tokens_only() {
    let mut server = mockito::Server::new_async().await;
    let factory = make_factory(&server);

    let body = "data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";

    server
        .mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer test-key")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .create_async()
        .await;

    let service = factory.create().unwrap();
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
async fn custom_headers_from_config_are_sent() {
    let mut server = mockito::Server::new_async().await;

    let config = ProviderConfig {
        name: "TestCustom",
        default_base_url: "https://example.com/v1/",
        chat_endpoint: "chat/completions",
        models_endpoint: "models",
        custom_headers: vec![("X-Custom-Header".to_owned(), "custom-value".to_owned())],
    };

    let factory = OpenAiCompatibleFactory::new(
        config,
        "test-model".to_owned(),
        Some(server.url()),
        "test-key".to_owned(),
        None,
        "test-custom".to_owned(),
    );

    server
        .mock("POST", "/chat/completions")
        .match_header("x-custom-header", "custom-value")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body("data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n")
        .create_async()
        .await;

    let service = factory.create().unwrap();
    let stream = service
        .chat_stream_with_tools(
            vec![LlmMessage::User {
                content: "hi".into(),
            }],
            vec![],
        )
        .await
        .unwrap();

    let _events: Vec<StreamEvent> = stream.filter_map(|r| async move { r.ok() }).collect().await;
}
