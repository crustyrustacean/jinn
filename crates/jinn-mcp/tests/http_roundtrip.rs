//! HTTP transport roundtrip: spawn → connect → tools/list → tools/call.
//!
//! Serves a real Streamable HTTP MCP endpoint in-process (via axum + rmcp's
//! `StreamableHttpService`), then drives `McpClient::connect_remote` through a
//! full lifecycle. This proves the HTTP client path end-to-end — handshake,
//! tool advertisement, and tool dispatch — against a genuine HTTP server,
//! complementing the failure/latency tests in `http_transport.rs`.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::missing_docs_in_private_items,
    reason = "test-only HTTP server fixture"
)]

use std::sync::Arc;
use std::time::Duration;

use jinn_mcp::client::McpClient;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestMethod, CallToolRequestParams, CallToolResponse, ContentBlock, ErrorData,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// A minimal MCP HTTP server advertising one `echo` tool (mirrors the in-memory
/// stub in `server_testkit.rs`, but served over real HTTP).
struct EchoServer;

impl ServerHandler for EchoServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::default())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tool = Tool::new(
            "echo",
            "Echoes the `message` argument back as text.",
            Arc::new(
                json!({
                    "type": "object",
                    "properties": { "message": { "type": "string" } },
                    "required": ["message"],
                })
                .as_object()
                .expect("valid object")
                .clone(),
            ),
        );
        Ok(ListToolsResult::with_all_items(vec![tool]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name.as_ref() != "echo" {
            return Err(ErrorData::method_not_found::<CallToolRequestMethod>());
        }
        let message = request
            .arguments
            .as_ref()
            .and_then(|args| args.get("message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("(no message)");
        let result =
            rmcp::model::CallToolResult::success(vec![ContentBlock::text(message.to_owned())]);
        Ok(result.into())
    }
}

/// Spawns an in-process Streamable HTTP MCP server on an ephemeral port,
/// returning the client URL and a cancellation token for shutdown.
async fn spawn_http_server() -> (String, CancellationToken) {
    let cancellation_token = CancellationToken::new();
    let service: StreamableHttpService<EchoServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(EchoServer),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener binds");
    let addr = listener.local_addr().expect("listener has an address");

    let shutdown_token = cancellation_token.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                shutdown_token.cancelled().await;
            })
            .await;
    });

    (format!("http://{addr}/mcp"), cancellation_token)
}

/// Extracts the first text content from a tool result.
fn first_text(result: &rmcp::model::CallToolResult) -> String {
    for block in &result.content {
        if let ContentBlock::Text(text) = block {
            return text.text.clone();
        }
    }
    String::new()
}

/// A full HTTP lifecycle — connect, list, call — completes end-to-end.
#[tokio::test]
async fn http_roundtrip_lists_and_calls_tools() {
    // Given a running in-process HTTP MCP server.
    let (url, shutdown) = spawn_http_server().await;

    // When connecting to it via the remote-HTTP path (no child process).
    let mut client = tokio::time::timeout(Duration::from_secs(5), McpClient::connect_remote(&url))
        .await
        .expect("connect should complete within 5s")
        .expect("connect should succeed against a live server");

    // Then tools/list advertises the `echo` tool.
    let tools = client.list_tools().await.expect("tools/list succeeds");
    assert!(
        tools.iter().any(|t| t.name == "echo"),
        "expected `echo` tool, got: {:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    // And tools/call returns the echoed message.
    let arguments = Some(
        json!({"message": "hello over http"})
            .as_object()
            .expect("valid object")
            .clone(),
    );
    let result = client
        .call_tool("echo", arguments)
        .await
        .expect("tools/call succeeds");
    assert_eq!(first_text(&result), "hello over http");

    client.shutdown().await;
    shutdown.cancel();
}

/// Cancelling the transport token flips `is_transport_closed()` to true.
/// This is the load-bearing contract the HTTP child-exit watcher relies on:
/// when the child dies, the watcher calls `cancel_token().cancel()`, and the
/// existing liveness watcher observes the close and publishes Dead.
#[tokio::test]
async fn cancel_token_cancels_the_transport() {
    // Given a connected client (remote path, but the transport cancellation
    // contract is the same as HTTP).
    let (url, shutdown) = spawn_http_server().await;
    let client = McpClient::connect_remote(&url)
        .await
        .expect("connect succeeds against the live server");

    // Then the transport is open before cancellation.
    let probe = client.liveness_probe();
    assert!(
        !probe.is_transport_closed(),
        "transport should be open before cancellation"
    );

    // When the cancellation token is triggered.
    client.cancel_token().cancel();

    // Then within a short window the transport reports closed.
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        while !probe.is_transport_closed() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .is_ok();
    assert!(closed, "transport should report closed after cancellation");

    shutdown.cancel();
}
