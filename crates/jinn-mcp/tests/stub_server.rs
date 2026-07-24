//! End-to-end integration test: spawn an in-process stub MCP server over an
//! in-memory duplex pipe and exercise the full client lifecycle through
//! `McpClient` — connect → list_tools → call_tool → shutdown.
//!
//! This validates the real `McpClient` against a real (if minimal) MCP server
//! implementation, exercising the JSON-RPC handshake, tool advertisement, and
//! tool dispatch paths without depending on an external process.
//!
//! The stub server advertises one tool, `echo`, which returns its `message`
//! argument verbatim as text content.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::map_err_ignore,
    clippy::missing_docs_in_private_items,
    reason = "test-only stub fixtures and harness"
)]

use std::sync::Arc;

use jinn_mcp::client::{McpClient, ServerCommand};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestMethod, CallToolRequestParams, CallToolResponse, ContentBlock, ErrorData,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::{RequestContext, RoleClient, RoleServer, ServiceExt};
use rmcp::transport::async_rw::AsyncRwTransport;
use serde_json::{Value, json};
use tokio::io::DuplexStream;

/// A minimal MCP server that advertises one `echo` tool.
///
/// All trait methods have defaults except `get_info`, `list_tools`, and
/// `call_tool`, which we override.
struct EchoServer;

impl EchoServer {
    fn echo_tool() -> Tool {
        Tool::new(
            "echo",
            "Echoes the `message` argument back as text.",
            Arc::new(
                json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string" }
                    },
                    "required": ["message"],
                })
                .as_object()
                .expect("valid object")
                .clone(),
            ),
        )
    }
}

impl ServerHandler for EchoServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::default())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![Self::echo_tool()]))
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
            .and_then(Value::as_str)
            .unwrap_or("(no message)");
        let result =
            rmcp::model::CallToolResult::success(vec![ContentBlock::text(message.to_owned())]);
        Ok(result.into())
    }
}

/// Wires an in-process `EchoServer` to one end of a duplex pipe and returns
/// the other end for the client to connect through.
fn spawn_stub_server() -> DuplexStream {
    let (server_io, client_io) = tokio::io::duplex(4096);
    // `serve` returns once the handshake completes; `waiting()` keeps the
    // server handling requests until the transport closes. Detach the task.
    tokio::spawn(async move {
        let Ok(running) = EchoServer.serve(server_io).await else {
            return;
        };
        let _ = running.waiting().await;
    });
    client_io
}

#[tokio::test]
async fn connect_lists_tools_calls_echo_and_shuts_down() {
    // Given a stub MCP server reachable over an in-memory duplex pipe.
    let client_io = spawn_stub_server();
    let (client_read, client_write) = tokio::io::split(client_io);

    // When connecting the real client.
    let mut client = McpClient::connect_with_transport(AsyncRwTransport::<
        RoleClient,
        tokio::io::ReadHalf<DuplexStream>,
        tokio::io::WriteHalf<DuplexStream>,
    >::new(client_read, client_write))
    .await
    .expect("client must connect to stub server");

    // Then `tools/list` advertises the `echo` tool.
    let tools = client.list_tools().await.expect("tools/list must succeed");
    let tool_name = tools
        .first()
        .map(|t| t.name.as_ref().to_owned())
        .unwrap_or_default();
    assert_eq!(tools.len(), 1, "stub advertises exactly one tool");
    assert_eq!(tool_name, "echo");

    // When calling `echo` with a message argument.
    let result = client
        .call_tool(
            "echo",
            Some(
                json!({ "message": "hello mcp" })
                    .as_object()
                    .expect("object")
                    .clone(),
            ),
        )
        .await
        .expect("tools/call must succeed");

    // Then the result echoes the message back as text.
    assert!(
        !result.is_error.unwrap_or(false),
        "echo should succeed, not report an error"
    );
    let text = result
        .content
        .into_iter()
        .find_map(|block| match block {
            ContentBlock::Text(t) => Some(t.text),
            _ => None,
        })
        .unwrap_or_default();
    assert_eq!(text, "hello mcp");

    // And the client shuts down without error (no panic, no hang).
    client.shutdown().await;
}

#[tokio::test]
async fn calling_an_unknown_tool_returns_a_protocol_error() {
    // Given a connected client with the echo server.
    let client_io = spawn_stub_server();
    let (client_read, client_write) = tokio::io::split(client_io);
    let client = McpClient::connect_with_transport(AsyncRwTransport::<
        RoleClient,
        tokio::io::ReadHalf<DuplexStream>,
        tokio::io::WriteHalf<DuplexStream>,
    >::new(client_read, client_write))
    .await
    .expect("client must connect");

    // When calling a tool the server does not advertise.
    let result = client.call_tool("does_not_exist", None).await;

    // Then the client surfaces a protocol-level error (Err, not a success).
    assert!(
        result.is_err(),
        "unknown tool must produce a client error, not a silent success"
    );
}

#[test]
fn server_command_carries_program_and_args() {
    // Given an excalimate-style stdio command.
    let cmd = ServerCommand {
        program: "npx".to_owned(),
        args: vec!["@excalimate/mcp-server".to_owned(), "--stdio".to_owned()],
    };

    // Then the fields round-trip (sanity check for the owned value the actor carries).
    assert_eq!(cmd.program, "npx");
    assert_eq!(cmd.args, vec!["@excalimate/mcp-server", "--stdio"]);
}
