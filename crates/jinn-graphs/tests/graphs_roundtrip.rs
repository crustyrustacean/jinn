//! `jinn mcp graphs` isolation and tool roundtrip over real HTTP.
//!
//! Serves the real [`jinn_graphs::run`] entrypoint (axum + rmcp's
//! `StreamableHttpService`) on an ephemeral port over a temp graphs dir,
//! then drives it with jinn's own `McpClient`. No `jinn.toml`, no
//! `providers.toml`, no session store: the subcommand must run without any
//! of them. Proves connect, tool advertisement, the full
//! create→transact→query cycle, persistence across server restart, and
//! minigraf's cross-handle lock refusal surfacing as a clean tool error.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::missing_docs_in_private_items,
    reason = "test-only HTTP server fixture"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// Integration test binaries never run the crate's `#[cfg(test)]` lib hook, so
// install the ring rustls provider here directly: reqwest is built with
// `rustls-no-provider` (see the workspace `Cargo.toml`) and panics with
// "No provider set" at `reqwest::Client` construction otherwise.
// `install_default` errors on the second call; the result is deliberately ignored.
#[ctor::ctor]
fn install_rustls_provider_for_tests() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

use jinn_mcp::client::McpClient;
use rmcp::model::ContentBlock;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// A graphs server bound to an ephemeral port, plus the URL to reach it.
///
/// Mirrors `jinn-mcp/tests/http_roundtrip.rs`: bind first (the port is known
/// before serving starts), then serve the real `GraphsServer` stack on the
/// bound listener. `jinn_graphs::run` itself owns its listener, so the test
/// mounts the identical service stack the entrypoint builds —
/// `GraphsServer::new` over the temp dir on axum at `/mcp` — which is the
/// same code path minus the listener plumbing.
async fn spawn_graphs_server(graphs_dir: PathBuf) -> (String, CancellationToken) {
    let server = jinn_graphs::GraphsServer::new(graphs_dir).expect("server builds");
    let service: StreamableHttpService<jinn_graphs::GraphsServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(server.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener binds");
    let addr = listener.local_addr().expect("listener has an address");

    let cancellation_token = CancellationToken::new();
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

/// Connects with retry so a just-bound server is not flaked on.
async fn connect(url: &str) -> McpClient {
    for _ in 0..50 {
        if let Ok(client) = McpClient::connect_remote(url, Vec::new()).await {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Test fixture: unreachable past the bounded retry.
    #[expect(
        clippy::panic,
        reason = "test fixture exhaustion is a hard stop"
    )]
    {
        panic!("client never connected to {url}");
    }
}

/// Calls a tool and returns its first text content block.
async fn call_text(client: &McpClient, name: &str, args: serde_json::Value) -> String {
    let result = client
        .call_tool(name, Some(args.as_object().expect("args object").clone()))
        .await
        .expect("tool call succeeds");
    result
        .content
        .first()
        .and_then(|block| {
            if let ContentBlock::Text(text) = block {
                Some(text.text.clone())
            } else {
                None
            }
        })
        .expect("a text content block")
}

#[rstest::rstest]
#[tokio::test]
async fn server_lists_exactly_the_four_graph_tools() {
    // Given a live graphs server over a temp dir.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let (url, shutdown) = spawn_graphs_server(dir.path().to_path_buf()).await;
    let mut client = connect(&url).await;

    // When listing tools.
    let tools = client.list_tools().await.expect("tools/list");

    // Then exactly the four graph tools are advertised.
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        vec!["create_graph", "list_graphs", "query", "transact"]
    );

    client.shutdown().await;
    shutdown.cancel();
}

#[rstest::rstest]
#[tokio::test]
async fn create_transact_query_cycle_roundtrips_over_http() {
    // Given a live graphs server.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let (url, shutdown) = spawn_graphs_server(dir.path().to_path_buf()).await;
    let mut client = connect(&url).await;

    // When creating, transacting, and querying a graph through the tools.
    call_text(&client, "create_graph", json!({ "name": "workflow-state" })).await;
    call_text(
        &client,
        "transact",
        json!({
            "graph": "workflow-state",
            "datalog": "(transact [[:step/1 :step/status :done]])"
        }),
    )
    .await;
    let rows = call_text(
        &client,
        "query",
        json!({
            "graph": "workflow-state",
            "datalog": "(query [:find ?s :where [?e :step/status ?s]])"
        }),
    )
    .await;

    // Then the written fact reads back.
    assert!(rows.contains("done"), "rows: {rows}");

    client.shutdown().await;
    shutdown.cancel();
}

#[rstest::rstest]
#[tokio::test]
async fn create_graph_on_existing_name_fails_without_touching_the_file() {
    // Given a live server with a created graph holding one fact.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let (url, shutdown) = spawn_graphs_server(dir.path().to_path_buf()).await;
    let mut client = connect(&url).await;
    call_text(&client, "create_graph", json!({ "name": "dup" })).await;
    call_text(
        &client,
        "transact",
        json!({ "graph": "dup", "datalog": "(transact [[:a :b \"kept\"]])" }),
    )
    .await;

    // When creating the same name again, the call fails with the collision
    // message riding the wire error (rmcp maps an ErrorData tool return to
    // a JSON-RPC -32603 on the client side).
    let result = client
        .call_tool(
            "create_graph",
            Some(json!({ "name": "dup" }).as_object().expect("o").clone()),
        )
        .await;

    // Then the error names the collision.
    let err = result.expect_err("second create must fail");
    let rendered = format!("{err:?}");
    assert!(rendered.contains("already exists"), "rendered: {rendered}");
    // And the original graph still serves its fact.
    let rows = call_text(
        &client,
        "query",
        json!({ "graph": "dup", "datalog": "(query [:find ?v :where [?e :b ?v]])" }),
    )
    .await;
    assert!(rows.contains("kept"), "rows: {rows}");

    client.shutdown().await;
    shutdown.cancel();
}

#[rstest::rstest]
#[tokio::test]
async fn transact_on_unknown_graph_names_create_graph_as_remedy_over_http() {
    // Given a live server with no graphs.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let (url, shutdown) = spawn_graphs_server(dir.path().to_path_buf()).await;
    let mut client = connect(&url).await;

    // When transacting into an unknown graph.
    let result = client
        .call_tool(
            "transact",
            Some(
                json!({ "graph": "ghost", "datalog": "(transact [[:a :b 1]])" })
                    .as_object()
                    .expect("o")
                    .clone(),
            ),
        )
        .await;

    // Then the error names create_graph as the remedy.
    let err = result.expect_err("unknown graph must be refused");
    let rendered = format!("{err:?}");
    assert!(rendered.contains("create_graph"), "rendered: {rendered}");

    client.shutdown().await;
    shutdown.cancel();
}

#[rstest::rstest]
#[tokio::test]
async fn list_graphs_reflects_created_graphs_over_http() {
    // Given a live server where two graphs were created.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let (url, shutdown) = spawn_graphs_server(dir.path().to_path_buf()).await;
    let mut client = connect(&url).await;
    call_text(&client, "create_graph", json!({ "name": "beta" })).await;
    call_text(&client, "create_graph", json!({ "name": "alpha" })).await;

    // When listing graphs.
    let listing = call_text(&client, "list_graphs", json!({})).await;

    // Then both names come back.
    assert!(listing.contains("alpha"), "listing: {listing}");
    assert!(listing.contains("beta"), "listing: {listing}");

    client.shutdown().await;
    shutdown.cancel();
}

#[rstest::rstest]
#[tokio::test]
async fn graphs_persist_across_a_server_restart() {
    // Given a server that created and wrote a graph, then stopped.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let graphs_dir = dir.path().to_path_buf();
    {
        let (url, shutdown) = spawn_graphs_server(graphs_dir.clone()).await;
        let mut client = connect(&url).await;
        call_text(&client, "create_graph", json!({ "name": "durable" })).await;
        call_text(
            &client,
            "transact",
            json!({ "graph": "durable", "datalog": "(transact [[:alice :task/name \"survives\"]])" }),
        )
        .await;
        client.shutdown().await;
        shutdown.cancel();
    }

    // When a fresh server over the same directory serves the same graph.
    let (url, shutdown) = spawn_graphs_server(graphs_dir).await;
    let mut client = connect(&url).await;
    let rows = call_text(
        &client,
        "query",
        json!({ "graph": "durable", "datalog": "(query [:find ?n :where [?e :task/name ?n]])" }),
    )
    .await;

    // Then the fact survived the restart.
    assert!(rows.contains("survives"), "rows: {rows}");

    client.shutdown().await;
    shutdown.cancel();
}

#[rstest::rstest]
#[tokio::test]
async fn lock_refusal_surfaces_as_clean_tool_error() {
    // Given a graph file held open by a direct minigraf handle (simulating
    // another jinn instance holding the same graph).
    let dir = tempfile::TempDir::new().expect("tempdir");
    let graphs_dir = dir.path().to_path_buf();
    std::fs::create_dir_all(&graphs_dir).expect("dir");
    let holder = minigraf::Minigraf::open(graphs_dir.join("locked.graph")).expect("held open");
    let (url, shutdown) = spawn_graphs_server(graphs_dir).await;
    let mut client = connect(&url).await;

    // When the server transacts into that graph.
    let result = client
        .call_tool(
            "transact",
            Some(
                json!({ "graph": "locked", "datalog": "(transact [[:a :b 1]])" })
                    .as_object()
                    .expect("o")
                    .clone(),
            ),
        )
        .await;

    // Then minigraf's lock refusal comes back promptly as a coded error —
    // never a hang, never corruption. (In one test process both handles are
    // in-process, so the STG-025 already-open guard fires; from a genuinely
    // separate process the OS-lock guard STG-026 fires with the same shape —
    // the store test `cross_process_lock_surfaces_as_coded_error_not_hang`
    // pins that both codes are accepted refusals.)
    let err = result.expect_err("lock must refuse");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("STG-025") || rendered.contains("STG-026"),
        "expected a coded lock refusal in: {rendered}"
    );

    client.shutdown().await;
    shutdown.cancel();
    drop(holder);
}
