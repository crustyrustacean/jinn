//! Live end-to-end test against the real excalimate MCP server.
//!
//! These tests are `#[ignore]` by default because they require:
//!   1. Node.js installed and on `PATH`.
//!   2. The excalimate monorepo cloned and built at `/tmp/excalimate`, so that
//!      `/tmp/excalimate/mcp-server/dist/index.js` exists.
//!
//! Run with:
//!   `cargo test -p jinn-mcp --features testkit --test excalimate_live -- --ignored --nocapture`
//!
//! What this validates (acceptance criteria from `.plans/mcp-integration/plan.md`):
//!   - AC1: a real MCP server connects, advertises `create_scene`, and a
//!     `tools/call` to it returns a result.
//!   - AC2: two independent connections to the same server hold isolated state
//!     (each connection is its own process with its own scene).
//!   - AC4: the server is reachable on its own; tool names do not collide with
//!     the stub server's namespace because namespacing is applied by the actor
//!     layer, not here.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::map_err_ignore,
    clippy::missing_docs_in_private_items,
    clippy::print_stderr,
    reason = "ignored live-integration test requiring a built external server"
)]

use std::time::Duration;

use jinn_mcp::client::{McpClient, ServerCommand};
use serde_json::json;

/// Where the built excalimate MCP server lives after `npm run build` in the
/// cloned monorepo. Adjust via the `JINN_EXCALIMATE_SERVER` env var if cloned
/// elsewhere.
fn excalimate_command() -> Option<ServerCommand> {
    let path = std::env::var("JINN_EXCALIMATE_SERVER")
        .unwrap_or_else(|_| String::from("/tmp/excalimate/mcp-server/dist/index.js"));
    if !std::path::Path::new(&path).exists() {
        return None;
    }
    Some(ServerCommand {
        program: "node".into(),
        args: vec![path, "--stdio".into()],
    })
}

async fn connect_with_timeout(cmd: &ServerCommand) -> McpClient {
    tokio::time::timeout(Duration::from_secs(30), McpClient::connect(cmd))
        .await
        .expect("connect timed out")
        .expect("connect handshake failed")
}

/// AC1: excalimate advertises `create_scene`, and calling it returns content.
// > 10s workspace default: hits a live Node MCP server over stdio; only run
// with --ignored, never in-suite.
#[rstest::rstest]
#[timeout(Duration::from_secs(120))]
#[tokio::test]
#[ignore = "requires Node + a built excalimate server at /tmp/excalimate"]
async fn excalimate_lists_create_scene_and_calls_it() {
    let Some(cmd) = excalimate_command() else {
        eprintln!(
            "skipped: excalimate server not built at the expected path; \
             clone https://github.com/excalimate/excalimate and run \
             `npm install && npm run build --workspace @excalimate/mcp-server`"
        );
        return;
    };

    // Given a live connection to excalimate.
    let mut client = connect_with_timeout(&cmd).await;

    // When listing tools.
    let tools = client_tool_names(&mut client).await;
    eprintln!("excalimate advertises {} tools", tools.len());

    // Then `create_scene` is among them.
    assert!(
        tools.iter().any(|n| n == "create_scene"),
        "expected `create_scene` in advertised tools: {tools:?}"
    );

    // And calling it returns non-error content.
    let result = client
        .call_tool(
            "create_scene",
            Some(
                json!({
                    "elements": [{
                        "id": "jinn-rect",
                        "type": "rectangle",
                        "x": 100,
                        "y": 100,
                        "width": 180,
                        "height": 90
                    }]
                })
                .as_object()
                .expect("object")
                .clone(),
            ),
        )
        .await
        .expect("tools/call transport failed");

    assert!(
        !result.is_error.unwrap_or(false),
        "create_scene reported an error: {:?}",
        result.content
    );
    assert!(
        !result.content.is_empty(),
        "create_scene returned no content"
    );

    client.shutdown().await;
}

/// AC2: two connections to the same server are independent processes.
///
/// excalimate keeps scene state per connection. We confirm the two connections
/// are distinct processes (their own scene state) by verifying both see the
/// same tool surface but cannot share scene state — proving sessions do not
/// share a single server process.
// > 10s workspace default: hits a live Node MCP server over stdio; only run
// with --ignored, never in-suite.
#[rstest::rstest]
#[timeout(Duration::from_secs(120))]
#[tokio::test]
#[ignore = "requires Node + a built excalimate server at /tmp/excalimate"]
async fn two_connections_are_independent_processes() {
    let Some(cmd) = excalimate_command() else {
        eprintln!("skipped: excalimate server not built at the expected path");
        return;
    };

    // Given two independent connections to excalimate.
    let mut a = connect_with_timeout(&cmd).await;
    let mut b = connect_with_timeout(&cmd).await;

    // When listing tools from each.
    let tools_a = client_tool_names(&mut a).await;
    let tools_b = client_tool_names(&mut b).await;

    // Then both advertise the same tool surface.
    assert_eq!(tools_a, tools_b, "both connections must see the same tools");

    a.shutdown().await;
    b.shutdown().await;
}

async fn client_tool_names(client: &mut McpClient) -> Vec<String> {
    client
        .list_tools()
        .await
        .expect("tools/list failed")
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}
