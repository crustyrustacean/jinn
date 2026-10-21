//! Handler-level tests for the overgraph MCP server.
//!
//! Drives the real server through a real [`McpClient`] over an in-memory
//! duplex pipe — the same harness `jinn-mcp/tests/stub_server.rs` uses —
//! against a real engine opened on a temp directory. Observable behavior
//! only: what tools are advertised, what a call returns, and that engine
//! failures come back as tool-level errors rather than panics.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code asserts with expect/panic and indexes fixtures for clear failures"
)]

use rmcp::model::ContentBlock;
use rmcp::service::{RoleClient, ServiceExt};
use rmcp::transport::async_rw::AsyncRwTransport;
use serde_json::{Value, json};
use tokio::io::DuplexStream;

use jinn_mcp::client::McpClient;

use jinn_mcp_overgraph::embedder::{Embedder, FakeEmbedder};

/// The vector configuration the server opens its databases with: the default
/// embedder model's dimension and metric, plus edge uniqueness (matching the
/// server's `db_options`). Tests that reopen a database by hand must match it.
fn server_db_options() -> overgraph::types::DbOptions {
    let model = jinn_mcp_overgraph::resolve_embed_model(None);
    overgraph::types::DbOptions {
        dense_vector: Some(overgraph::types::DenseVectorConfig {
            dimension: model.dimension(),
            metric: model.metric(),
            hnsw: overgraph::types::HnswConfig::default(),
        }),
        edge_uniqueness: true,
        ..overgraph::types::DbOptions::default()
    }
}

/// Spawns the overgraph server over one end of a duplex pipe with the given
/// engine and embedder, and returns the client end (handshake complete).
async fn connect_with_engine(
    engine: overgraph::DatabaseEngine,
    embedder: Arc<dyn Embedder>,
) -> McpClient {
    let server = jinn_mcp_overgraph::server::OvergraphMcpServer::new(Arc::new(engine), embedder);
    let (server_io, client_io) = tokio::io::duplex(4096);
    // `serve` returns once the handshake completes; `waiting()` keeps the
    // server handling requests until the transport closes. Detach the task.
    tokio::spawn(async move {
        let Ok(running) = ServiceExt::serve(server, server_io).await else {
            return;
        };
        let _ = running.waiting().await;
    });
    let (client_read, client_write) = tokio::io::split(client_io);
    McpClient::connect_with_transport(AsyncRwTransport::<
        RoleClient,
        tokio::io::ReadHalf<DuplexStream>,
        tokio::io::WriteHalf<DuplexStream>,
    >::new(client_read, client_write))
    .await
    .expect("client must connect to the overgraph server")
}

/// Spawns the overgraph server with a fresh engine on `dir` and a fake
/// embedder standing in for the local model.
async fn connect_server(dir: &std::path::Path) -> McpClient {
    let engine = overgraph::DatabaseEngine::open(dir, &server_db_options())
        .expect("engine opens on a temp dir");
    connect_with_engine(engine, Arc::new(FakeEmbedder::available(384))).await
}

/// Spawns the overgraph server on an engine that has already been closed —
/// the `graph_docs` arm never touches the engine, so it must still serve.
async fn connect_server_with_closed_engine(dir: &std::path::Path) -> McpClient {
    let engine = overgraph::DatabaseEngine::open(dir, &server_db_options())
        .expect("engine opens on a temp dir");
    engine.close().expect("engine closes");
    connect_with_engine(engine, Arc::new(FakeEmbedder::available(384))).await
}

/// Extracts the text payload of a tool response and parses it as JSON when
/// it parses (the structured tools return JSON text; error text stays raw).
fn response_json(result: &rmcp::model::CallToolResult) -> Value {
    let text = result
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

use std::sync::Arc;

#[rstest::rstest]
#[tokio::test]
async fn list_tools_advertises_exactly_six_graph_tools() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When listing tools.
    let tools = client.list_tools().await.expect("tools/list must succeed");

    // Then exactly the six graph tools are advertised, by name.
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        [
            "graph_query",
            "graph_upsert_node",
            "graph_upsert_edge",
            "graph_vector_search",
            "graph_ingest_file",
            "graph_docs"
        ]
    );
    // And each carries a JSON-schema object.
    for tool in &tools {
        assert!(
            tool.input_schema.contains_key("type"),
            "{} has a schema",
            tool.name
        );
    }

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn call_tool_rejects_unknown_tool_names() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When calling a tool that does not exist.
    let result = client
        .call_tool(
            "graph_delete",
            Some(json!({}).as_object().expect("obj").clone()),
        )
        .await;

    // Then the call fails (protocol-level method/params error, not a panic).
    assert!(result.is_err(), "unknown tool must not succeed");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_upsert_node_creates_a_queryable_node() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When upserting a node with properties.
    let result = client
        .call_tool(
            "graph_upsert_node",
            Some(
                json!({"label": "Person", "key": "alice", "props": {"age": 30}})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("call succeeds");
    let payload = response_json(&result);

    // Then the response carries the node id.
    assert!(!result.is_error.unwrap_or(false), "upsert must not error");
    assert!(payload["node_id"].is_u64(), "node id returned: {payload}");

    // And the node round-trips through a GQL query.
    let result = client
        .call_tool(
            "graph_query",
            Some(
                json!({"query": "MATCH (n:Person) RETURN n"})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("query succeeds");
    assert!(!result.is_error.unwrap_or(false), "query must not error");
    let payload = response_json(&result);
    assert_eq!(payload["rows"][0]["n"]["key"], json!("alice"));
    assert_eq!(payload["rows"][0]["n"]["props"]["age"], json!(30));

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_upsert_node_is_idempotent_for_the_same_key() {
    // Given a node already upserted once.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    let args = || {
        json!({"label": "Person", "key": "alice"})
            .as_object()
            .expect("obj")
            .clone()
    };
    client
        .call_tool("graph_upsert_node", Some(args()))
        .await
        .expect("first");

    // When upserting the same (label, key) again.
    let result = client
        .call_tool("graph_upsert_node", Some(args()))
        .await
        .expect("second");
    let second = response_json(&result);

    // Then the same node id comes back — update, not duplicate.
    assert_eq!(second, json!({"node_id": 1}));

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_upsert_edge_traverses_through_gql() {
    // Given two upserted nodes.
    let (_dir, mut client) = {
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let client = connect_server(dir.path()).await;
        for key in ["alice", "bob"] {
            client
                .call_tool(
                    "graph_upsert_node",
                    Some(
                        json!({"label": "Person", "key": key})
                            .as_object()
                            .expect("obj")
                            .clone(),
                    ),
                )
                .await
                .expect("node upsert");
        }
        (dir, client)
    };

    // When creating an edge between them.
    let result = client
        .call_tool(
            "graph_upsert_edge",
            Some(
                json!({
                    "from_label": "Person", "from_key": "alice",
                    "to_label": "Person", "to_key": "bob",
                    "label": "KNOWS",
                    "props": {"since": 2020}
                })
                .as_object()
                .expect("obj")
                .clone(),
            ),
        )
        .await
        .expect("edge call succeeds");
    let payload = response_json(&result);

    // Then the edge id is returned.
    assert!(
        !result.is_error.unwrap_or(false),
        "edge upsert must not error"
    );
    assert!(payload["edge_id"].is_u64(), "edge id returned: {payload}");

    // And the edge traverses from alice to bob through GQL.
    let result = client
        .call_tool(
            "graph_query",
            Some(
                json!({"query": "MATCH (a:Person)-[e:KNOWS]->(b:Person) RETURN b"})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("traversal query");
    assert!(!result.is_error.unwrap_or(false));
    let payload = response_json(&result);
    assert_eq!(payload["rows"][0]["b"]["key"], json!("bob"));

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_upsert_edge_reports_missing_endpoints_as_tool_errors() {
    // Given a server whose graph has no nodes.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When creating an edge to a nonexistent endpoint.
    let result = client
        .call_tool(
            "graph_upsert_edge",
            Some(
                json!({
                    "from_label": "Person", "from_key": "ghost",
                    "to_label": "Person", "to_key": "bob",
                    "label": "KNOWS"
                })
                .as_object()
                .expect("obj")
                .clone(),
            ),
        )
        .await
        .expect("call returns, does not panic");

    // Then the failure is a tool-level error naming the endpoint.
    assert!(
        result.is_error.unwrap_or(false),
        "missing endpoint is a tool error"
    );
    let text = result
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    assert!(text.contains("ghost"), "error names the endpoint: {text}");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_query_surfaces_malformed_gql_as_tool_errors() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When submitting GQL the engine cannot parse.
    let result = client
        .call_tool(
            "graph_query",
            Some(
                json!({"query": "THIS IS NOT GQL"})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("call returns, does not panic");

    // Then the failure is a tool-level error the model can read.
    assert!(
        result.is_error.unwrap_or(false),
        "malformed GQL is a tool error"
    );

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_query_binds_json_params_into_the_statement() {
    // Given a node whose key we know.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    client
        .call_tool(
            "graph_upsert_node",
            Some(
                json!({"label": "Person", "key": "alice"})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("node upsert");

    // When querying with a JSON parameter (the key property is `elementKey`).
    let result = client
        .call_tool(
            "graph_query",
            Some(
                json!({
                    "query": "MATCH (n:Person {elementKey: $who}) RETURN n",
                    "params": {"who": "alice"}
                })
                .as_object()
                .expect("obj")
                .clone(),
            ),
        )
        .await
        .expect("query succeeds");

    // Then the parameter bound and the row matched.
    assert!(!result.is_error.unwrap_or(false));
    let payload = response_json(&result);
    assert_eq!(payload["rows_returned"], json!(1));
    assert_eq!(payload["rows"][0]["n"]["key"], json!("alice"));

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_docs_without_arguments_serves_the_whole_reference() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When calling graph_docs with no arguments at all.
    let result = client
        .call_tool("graph_docs", None)
        .await
        .expect("call succeeds");

    // Then the whole vendored reference comes back verbatim behind the
    // provenance line — byte-equal to the embedded constant.
    assert!(!result.is_error.unwrap_or(false), "docs must not error");
    let text = response_json(&result)
        .as_str()
        .expect("text content")
        .to_owned();
    assert_eq!(
        text,
        format!(
            "{}\n\n{}",
            jinn_mcp_overgraph::docs::GQL_SUBSET_PROVENANCE,
            jinn_mcp_overgraph::docs::GQL_SUBSET
        )
    );

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_docs_section_returns_that_section_verbatim() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When calling graph_docs for one named section.
    let result = client
        .call_tool(
            "graph_docs",
            Some(
                json!({"section": "Read Syntax"})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("call succeeds");

    // Then exactly that section comes back, header included and nothing else.
    assert!(!result.is_error.unwrap_or(false), "section must not error");
    let text = response_json(&result)
        .as_str()
        .expect("text content")
        .to_owned();
    assert_eq!(
        text,
        jinn_mcp_overgraph::docs::section(jinn_mcp_overgraph::docs::GQL_SUBSET, "Read Syntax")
            .expect("the section exists in the vendored doc")
    );
    assert!(text.starts_with("## Read Syntax\n"));
    assert!(!text.contains("## Mutation Syntax"), "one section only");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_docs_unknown_section_errors_with_the_valid_names() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When calling graph_docs with a section title nothing carries.
    let result = client
        .call_tool(
            "graph_docs",
            Some(json!({"section": "Nope"}).as_object().expect("obj").clone()),
        )
        .await
        .expect("call returns, does not panic");

    // Then the failure is a tool-level error listing the valid sections.
    assert!(
        result.is_error.unwrap_or(false),
        "unknown section is a tool error"
    );
    let text = response_json(&result)
        .as_str()
        .expect("text content")
        .to_owned();
    assert!(text.contains("no section titled `Nope`"), "{text}");
    assert!(text.contains("Read Syntax"), "{text}");
    assert!(text.contains("Examples"), "{text}");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_docs_schema_advertises_no_required_arguments() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When listing tools and finding graph_docs.
    let tools = client.list_tools().await.expect("tools/list");
    let docs_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "graph_docs")
        .expect("graph_docs is advertised");

    // Then its schema requires nothing — the whole document is the default.
    let schema = serde_json::to_value(&*docs_tool.input_schema).expect("schema is JSON");
    let required = schema.get("required");
    assert!(
        required.is_none_or(|r| r.as_array().is_none_or(Vec::is_empty)),
        "no required args: {schema}"
    );
    assert!(
        schema["properties"]["section"]["type"] == json!("string"),
        "optional section arg described: {schema}"
    );

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn graph_docs_serves_with_a_closed_engine() {
    // Given a server whose engine has already been closed.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server_with_closed_engine(dir.path()).await;

    // When calling graph_docs.
    let result = client
        .call_tool("graph_docs", None)
        .await
        .expect("call succeeds");

    // Then the reference still serves — the docs arm never touches the engine.
    assert!(
        !result.is_error.unwrap_or(false),
        "docs serve without a live engine"
    );
    let text = response_json(&result)
        .as_str()
        .expect("text content")
        .to_owned();
    assert!(text.contains("# GQL"), "{text}");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[test]
fn resolve_db_dir_prefers_the_explicit_override() {
    // Given an explicit --db path.
    let requested = std::path::PathBuf::from("/explicit/db");

    // When resolving the database directory.
    let resolved = jinn_mcp_overgraph::resolve_db_dir(Some(requested.clone()));

    // Then the override wins.
    assert_eq!(resolved, requested);
}

#[rstest::rstest]
#[tokio::test]
async fn embed_text_upsert_then_vector_search_finds_node() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When upserting a node with embed_text.
    let upsert = client
        .call_tool(
            "graph_upsert_node",
            Some(
                json!({
                    "label": "Doc",
                    "key": "rust-guide",
                    "embed_text": "how to build a rust project with cargo"
                })
                .as_object()
                .expect("obj")
                .clone(),
            ),
        )
        .await
        .expect("upsert call succeeds");
    assert!(
        !upsert.is_error.unwrap_or(false),
        "embed upsert must not error"
    );

    // Then a vector search on the same text finds that node, enriched.
    let search = client
        .call_tool(
            "graph_vector_search",
            Some(
                json!({"query": "how to build a rust project with cargo", "k": 5})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("search call succeeds");
    assert!(
        !search.is_error.unwrap_or(false),
        "vector search must not error"
    );
    let payload = response_json(&search);
    let hits = payload["hits"].as_array().expect("hits array");
    assert_eq!(hits.len(), 1, "exactly the one embedded node: {payload}");
    assert_eq!(hits[0]["key"], json!("rust-guide"));
    assert_eq!(hits[0]["labels"], json!(["Doc"]));
    assert!(hits[0]["score"].is_number(), "score present: {payload}");
    assert!(hits[0]["node_id"].is_u64(), "node_id present: {payload}");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn upsert_without_embed_text_stays_pure_graph() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    client
        .call_tool(
            "graph_upsert_node",
            Some(
                json!({"label": "Doc", "key": "plain", "props": {"topic": "rust"}})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("upsert succeeds");

    // When searching for text that would match the node if it had been
    // embedded with the same text.
    let search = client
        .call_tool(
            "graph_vector_search",
            Some(json!({"query": "rust"}).as_object().expect("obj").clone()),
        )
        .await
        .expect("search call succeeds");

    // Then the search succeeds but finds nothing — no embedding was stored.
    assert!(!search.is_error.unwrap_or(false), "search must not error");
    let payload = response_json(&search);
    assert_eq!(
        payload["hits"].as_array().map(Vec::len),
        Some(0),
        "no embedded node, no hits: {payload}"
    );
    // And the node itself is still fully queryable through GQL.
    let query = client
        .call_tool(
            "graph_query",
            Some(
                json!({"query": "MATCH (n:Doc) RETURN n"})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("query succeeds");
    let payload = response_json(&query);
    assert_eq!(payload["rows"][0]["n"]["key"], json!("plain"));

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn vector_search_respects_k_and_label_filter() {
    // Given three embedded nodes, two in one label and one in another.
    let (_dir, mut client) = {
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let client = connect_server(dir.path()).await;
        for (label, key) in [("Doc", "alpha"), ("Doc", "beta"), ("Person", "gamma")] {
            client
                .call_tool(
                    "graph_upsert_node",
                    Some(
                        json!({"label": label, "key": key, "embed_text": "shared topic text"})
                            .as_object()
                            .expect("obj")
                            .clone(),
                    ),
                )
                .await
                .expect("node upsert");
        }
        (dir, client)
    };

    // When searching with a label filter naming only Doc.
    let search = client
        .call_tool(
            "graph_vector_search",
            Some(
                json!({"query": "shared topic text", "labels": ["Doc"]})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("search call succeeds");
    let payload = response_json(&search);
    let hits = payload["hits"].as_array().expect("hits array");

    // Then only the Doc-labeled nodes come back.
    assert_eq!(hits.len(), 2, "label filter applies: {payload}");
    assert!(
        hits.iter().all(|hit| hit["labels"] == json!(["Doc"])),
        "every hit is a Doc: {payload}"
    );

    // And when the same query is searched with k smaller than the match count.
    let search = client
        .call_tool(
            "graph_vector_search",
            Some(
                json!({"query": "shared topic text", "k": 1})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("search call succeeds");
    let payload = response_json(&search);

    // Then k bounds the result count.
    assert_eq!(
        payload["hits"].as_array().map(Vec::len),
        Some(1),
        "k bounds the hits: {payload}"
    );

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn empty_embed_text_is_rejected_as_invalid_params() {
    // Given a client connected to the overgraph server.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;

    // When upserting with an empty embed_text.
    let result = client
        .call_tool(
            "graph_upsert_node",
            Some(
                json!({"label": "Doc", "key": "blank", "embed_text": "   "})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await;

    // Then the call fails at the protocol level (invalid params), not as a
    // tool error — the request never reaches the engine or the embedder.
    assert!(result.is_err(), "empty embed_text must be invalid_params");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn missing_model_and_network_fails_as_tool_error() {
    // Given a client connected to a server whose embedder is unavailable
    // (cold cache, no network).
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let engine =
        overgraph::DatabaseEngine::open(dir.path(), &server_db_options()).expect("engine opens");
    let mut client = connect_with_engine(engine, Arc::new(FakeEmbedder::unavailable())).await;

    // When upserting with embed_text.
    let upsert = client
        .call_tool(
            "graph_upsert_node",
            Some(
                json!({"label": "Doc", "key": "offline", "embed_text": "text"})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("call returns, does not panic");

    // Then the failure is a tool-level error naming the download.
    assert!(
        upsert.is_error.unwrap_or(false),
        "embed failure is a tool error"
    );
    let text = response_json(&upsert)
        .as_str()
        .expect("error text")
        .to_owned();
    assert!(text.contains("download failed"), "{text}");

    // And the node was not created — the embed happens before the engine call.
    let query = client
        .call_tool(
            "graph_query",
            Some(
                json!({"query": "MATCH (n:Doc) RETURN n"})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("query succeeds");
    let payload = response_json(&query);
    assert_eq!(payload["rows_returned"], json!(0), "no partial upsert");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[test]
fn graph_only_db_upgrades_in_place_on_reopen_with_vectors() {
    // Given a database created without vector config, holding one node.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let (path, node_id) = {
        let engine =
            overgraph::DatabaseEngine::open(dir.path(), &overgraph::types::DbOptions::default())
                .expect("graph-only engine opens");
        let id = engine
            .upsert_node(
                "Doc",
                "legacy",
                overgraph::types::UpsertNodeOptions::default(),
            )
            .expect("node upserts");
        engine.close().expect("engine closes");
        (dir.path().to_path_buf(), id)
    };

    // When reopening it through the server's vector-aware options.
    let engine = overgraph::DatabaseEngine::open(&path, &server_db_options())
        .expect("reopen with vector config succeeds");

    // Then the old node is still there, and a vector write lands.
    let stored = engine
        .get_node(node_id)
        .expect("read")
        .expect("legacy node survives the upgrade");
    assert_eq!(stored.key, "legacy");
    let model = jinn_mcp_overgraph::resolve_embed_model(None);
    engine
        .upsert_node(
            "Doc",
            "vectorized",
            overgraph::types::UpsertNodeOptions {
                dense_vector: Some(vec![0.5; model.dimension() as usize]),
                ..overgraph::types::UpsertNodeOptions::default()
            },
        )
        .expect("vector write lands on the upgraded db");
    let hits = engine
        .vector_search(&overgraph::types::VectorSearchRequest {
            mode: overgraph::types::VectorSearchMode::Dense,
            dense_query: Some(vec![0.5; model.dimension() as usize]),
            sparse_query: None,
            k: 10,
            label_filter: None,
            ef_search: None,
            scope: None,
            dense_weight: None,
            sparse_weight: None,
            fusion_mode: None,
        })
        .expect("vector search works on the upgraded db");
    assert_eq!(hits.len(), 1, "the vectorized node is found");

    engine.close().expect("engine closes");
}

#[rstest::rstest]
#[test]
fn dimension_mismatch_fails_open_naming_both() {
    // Given a database locked to the default model's dimension.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let path = dir.path().to_path_buf();
    {
        let engine = overgraph::DatabaseEngine::open(&path, &server_db_options()).expect("opens");
        engine.close().expect("closes");
    }

    // When reopening with a different dimension (a 768-dim model).
    let other = overgraph::types::DbOptions {
        dense_vector: Some(overgraph::types::DenseVectorConfig {
            dimension: 768,
            metric: overgraph::types::DenseMetric::Cosine,
            hnsw: overgraph::types::HnswConfig::default(),
        }),
        ..overgraph::types::DbOptions::default()
    };
    let result = overgraph::DatabaseEngine::open(&path, &other);

    // Then the open fails, and the error names both configurations.
    // (`expect_err` needs `T: Debug`, which `DatabaseEngine` does not carry,
    // so the Err is pulled out by pattern.)
    let Err(err) = result else {
        panic!("mismatched reopen must fail");
    };
    let rendered = err.to_string();
    assert!(
        rendered.contains("mismatch"),
        "error names the mismatch: {rendered}"
    );
    assert!(
        rendered.contains("768"),
        "error names the requested dimension: {rendered}"
    );
    assert!(
        rendered.contains("384"),
        "error names the persisted dimension: {rendered}"
    );
}

/// Writes `content` to a temp file and returns its path (the ingest tool
/// reads from the filesystem, so fixtures go through `tempfile`).
fn batch_file(dir: &tempfile::TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).expect("fixture file writes");
    path
}

/// Ingest arguments naming `path`.
fn ingest_args(path: &std::path::Path) -> serde_json::Map<String, Value> {
    json!({"path": path.to_string_lossy()})
        .as_object()
        .expect("obj")
        .clone()
}

/// Runs one GQL query and returns the parsed response payload.
async fn query_json(client: &mut McpClient, query: &str) -> Value {
    let result = client
        .call_tool(
            "graph_query",
            Some(json!({"query": query}).as_object().expect("obj").clone()),
        )
        .await
        .expect("query succeeds");
    assert!(!result.is_error.unwrap_or(false), "query must not error");
    response_json(&result)
}

#[rstest::rstest]
#[tokio::test]
async fn ingest_file_processes_mixed_nodes_and_edges_in_any_order() {
    // Given a batch file whose edge lines precede the node lines they
    // reference, plus a third node living only in the database.
    let (_dir, mut client, path) = {
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let client = connect_server(dir.path()).await;
        client
            .call_tool(
                "graph_upsert_node",
                Some(
                    json!({"label": "Team", "key": "platform"})
                        .as_object()
                        .expect("obj")
                        .clone(),
                ),
            )
            .await
            .expect("pre-existing node");
        let path = batch_file(
            &dir,
            "mixed.jsonl",
            concat!(
                r#"{"edge": {"label": "KNOWS", "from": "Person:alice", "to": "Person:bob", "props": {"since": 2020}}}"#,
                "\n",
                r#"{"edge": {"label": "JOINS", "from": "Person:alice", "to": "Team:platform"}}"#,
                "\n",
                r#"{"node": {"label": "Person", "key": "alice", "props": {"age": 30}}}"#,
                "\n",
                r#"{"node": {"label": "Person", "key": "bob"}}"#,
                "\n",
            ),
        );
        (dir, client, path)
    };

    // When ingesting the file in one call.
    let result = client
        .call_tool("graph_ingest_file", Some(ingest_args(&path)))
        .await
        .expect("ingest call succeeds");
    assert!(
        !result.is_error.unwrap_or(false),
        "order-independent ingest must not error"
    );

    // Then every node and every edge is in the graph: alice traverses to
    // both bob and the pre-existing platform team.
    let payload = query_json(
        &mut client,
        "MATCH (a:Person)-[e]->(b) RETURN elementKey(b) AS who ORDER BY who",
    )
    .await;
    let who: Vec<&str> = payload["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["who"].as_str().expect("string"))
        .collect();
    assert_eq!(
        who,
        vec!["bob", "platform"],
        "both edges resolved: {payload}"
    );

    // And alice's props landed.
    let payload = query_json(
        &mut client,
        "MATCH (n:Person {elementKey: 'alice'}) RETURN n",
    )
    .await;
    assert_eq!(payload["rows"][0]["n"]["props"]["age"], json!(30));

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn ingest_file_aborts_when_an_edge_names_a_missing_node() {
    // Given a batch file whose only edge names a node declared nowhere.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    let path = batch_file(
        &dir,
        "dangling.jsonl",
        concat!(
            r#"{"node": {"label": "Person", "key": "alice"}}"#,
            "\n",
            r#"{"edge": {"label": "KNOWS", "from": "Person:alice", "to": "Person:bob"}}"#,
            "\n",
        ),
    );

    // When ingesting the file.
    let result = client
        .call_tool("graph_ingest_file", Some(ingest_args(&path)))
        .await
        .expect("call returns, does not panic");

    // Then the failure is a tool-level error naming the endpoint and line,
    // and stating nothing was written.
    assert!(
        result.is_error.unwrap_or(false),
        "missing endpoint must be a tool error"
    );
    let text = response_json(&result)
        .as_str()
        .expect("error text")
        .to_owned();
    assert!(text.contains("Person:bob"), "names the endpoint: {text}");
    assert!(
        text.contains("line 2"),
        "names the referencing line: {text}"
    );
    assert!(text.contains("nothing was written"), "{text}");

    // And the graph is untouched — zero nodes, zero edges, no partial write.
    let payload = query_json(&mut client, "MATCH (n:Person) RETURN count(*) AS total").await;
    assert_eq!(payload["rows"][0]["total"], json!(0), "{payload}");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn ingest_file_embeds_embed_text_and_vector_search_finds_it() {
    // Given a batch file whose node carries embed_text.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    let path = batch_file(
        &dir,
        "embedded.jsonl",
        concat!(
            r#"{"node": {"label": "Doc", "key": "rust-guide", "embed_text": "how to build a rust project with cargo"}}"#,
            "\n",
        ),
    );

    // When ingesting it.
    let result = client
        .call_tool("graph_ingest_file", Some(ingest_args(&path)))
        .await
        .expect("ingest call succeeds");
    let payload = response_json(&result);
    assert!(!result.is_error.unwrap_or(false), "{payload}");

    // Then the response counts the embedding.
    assert_eq!(payload["embedded"], json!(1), "{payload}");

    // And a vector search on the same text finds the ingested node.
    let search = client
        .call_tool(
            "graph_vector_search",
            Some(
                json!({"query": "how to build a rust project with cargo"})
                    .as_object()
                    .expect("obj")
                    .clone(),
            ),
        )
        .await
        .expect("search call succeeds");
    assert!(!search.is_error.unwrap_or(false));
    let payload = response_json(&search);
    let hits = payload["hits"].as_array().expect("hits array");
    assert_eq!(hits.len(), 1, "the embedded node is found: {payload}");
    assert_eq!(hits[0]["key"], json!("rust-guide"));

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn ingest_file_reports_counts_not_ids() {
    // Given a small batch file.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    let path = batch_file(
        &dir,
        "counts.jsonl",
        concat!(
            r#"{"node": {"label": "Person", "key": "alice"}}"#,
            "\n",
            r#"{"node": {"label": "Person", "key": "bob"}}"#,
            "\n",
            r#"{"edge": {"label": "KNOWS", "from": "Person:alice", "to": "Person:bob"}}"#,
            "\n",
        ),
    );

    // When ingesting it.
    let result = client
        .call_tool("graph_ingest_file", Some(ingest_args(&path)))
        .await
        .expect("ingest call succeeds");
    let payload = response_json(&result);

    // Then the response carries exactly the count/timing keys — and no id
    // array anywhere (ids resolve through graph_query, not the response).
    assert_eq!(payload["nodes_upserted"], json!(2), "{payload}");
    assert_eq!(payload["edges_upserted"], json!(1), "{payload}");
    assert_eq!(payload["embedded"], json!(0), "{payload}");
    assert_eq!(payload["engine_batches"], json!(2), "{payload}");
    assert!(payload["elapsed_ms"].is_u64(), "timing present: {payload}");
    let object = payload.as_object().expect("response is an object");
    for key in object.keys() {
        assert!(
            !key.contains("id"),
            "no id-carrying key in the response: {key}"
        );
    }

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn ingest_file_reports_the_offending_line_on_malformed_input() {
    // Given a batch file whose third line is not JSON.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    let path = batch_file(
        &dir,
        "broken.jsonl",
        concat!(
            r#"{"node": {"label": "Person", "key": "alice"}}"#,
            "\n",
            r#"{"node": {"label": "Person", "key": "bob"}}"#,
            "\n",
            "this line is not json",
            "\n",
        ),
    );

    // When ingesting it.
    let result = client
        .call_tool("graph_ingest_file", Some(ingest_args(&path)))
        .await
        .expect("call returns, does not panic");

    // Then the failure is a tool-level error naming line 3.
    assert!(
        result.is_error.unwrap_or(false),
        "malformed input must be a tool error"
    );
    let text = response_json(&result)
        .as_str()
        .expect("error text")
        .to_owned();
    assert!(text.contains("line 3"), "names the offending line: {text}");
    assert!(text.contains("invalid JSON"), "{text}");

    // And nothing was written — parsing completes before any engine call.
    let payload = query_json(&mut client, "MATCH (n:Person) RETURN count(*) AS total").await;
    assert_eq!(
        payload["rows"][0]["total"],
        json!(0),
        "fail-fast, zero writes"
    );

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn ingest_file_chunks_by_batch_size() {
    // Given a five-node file ingested with batch_size 2.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    let lines: Vec<String> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|key| format!(r#"{{"node": {{"label": "Person", "key": "{key}"}}}}"#))
        .collect();
    let path = batch_file(&dir, "chunked.jsonl", &format!("{}\n", lines.join("\n")));

    // When ingesting with an explicit batch_size.
    let args = json!({"path": path.to_string_lossy(), "batch_size": 2})
        .as_object()
        .expect("obj")
        .clone();
    let result = client
        .call_tool("graph_ingest_file", Some(args))
        .await
        .expect("ingest call succeeds");
    let payload = response_json(&result);
    assert!(!result.is_error.unwrap_or(false), "{payload}");

    // Then the batching is observable: 5 nodes / 2 = 3 node chunks, and no
    // edges means no edge chunks.
    assert_eq!(payload["engine_batches"], json!(3), "{payload}");
    assert_eq!(payload["nodes_upserted"], json!(5), "{payload}");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn ingest_file_rejects_out_of_range_batch_sizes_as_invalid_params() {
    // Given a connected client and a valid batch file.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    let path = batch_file(
        &dir,
        "ok.jsonl",
        concat!(r#"{"node": {"label": "Person", "key": "alice"}}"#, "\n"),
    );

    // When ingesting with a batch_size above the hard cap.
    let args = json!({"path": path.to_string_lossy(), "batch_size": 4097})
        .as_object()
        .expect("obj")
        .clone();
    let result = client.call_tool("graph_ingest_file", Some(args)).await;

    // Then the call fails at the protocol level (invalid params), before
    // the file is even read.
    assert!(
        result.is_err(),
        "out-of-range batch_size must be invalid_params"
    );

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn ingest_file_names_the_path_when_it_cannot_read_the_file() {
    // Given a connected client and a path no file exists at.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    let path = dir.path().join("nope.jsonl");

    // When ingesting it.
    let result = client
        .call_tool("graph_ingest_file", Some(ingest_args(&path)))
        .await
        .expect("call returns, does not panic");

    // Then the failure is a tool-level error naming the path.
    assert!(
        result.is_error.unwrap_or(false),
        "unreadable file must be a tool error"
    );
    let text = response_json(&result)
        .as_str()
        .expect("error text")
        .to_owned();
    assert!(text.contains("nope.jsonl"), "error names the path: {text}");

    // And the client shuts down without error.
    client.shutdown().await;
}

#[rstest::rstest]
#[tokio::test]
async fn ingest_file_is_idempotent_across_repeated_runs() {
    // Given a batch file ingested once.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let mut client = connect_server(dir.path()).await;
    let path = batch_file(
        &dir,
        "same.jsonl",
        concat!(
            r#"{"node": {"label": "Person", "key": "alice"}}"#,
            "\n",
            r#"{"edge": {"label": "KNOWS", "from": "Person:alice", "to": "Person:bob"}}"#,
            "\n",
            r#"{"node": {"label": "Person", "key": "bob"}}"#,
            "\n",
        ),
    );
    client
        .call_tool("graph_ingest_file", Some(ingest_args(&path)))
        .await
        .expect("first ingest");

    // When ingesting the same file again.
    let result = client
        .call_tool("graph_ingest_file", Some(ingest_args(&path)))
        .await
        .expect("second ingest");
    assert!(!result.is_error.unwrap_or(false), "re-run must not error");

    // Then the graph still holds exactly two nodes and one edge — upserts,
    // not duplicates.
    let payload = query_json(&mut client, "MATCH (n:Person) RETURN count(*) AS total").await;
    assert_eq!(payload["rows"][0]["total"], json!(2), "nodes stay 2");
    let payload = query_json(
        &mut client,
        "MATCH (:Person)-[e:KNOWS]->(:Person) RETURN count(*) AS total",
    )
    .await;
    assert_eq!(payload["rows"][0]["total"], json!(1), "edges stay 1");

    // And the client shuts down without error.
    client.shutdown().await;
}
