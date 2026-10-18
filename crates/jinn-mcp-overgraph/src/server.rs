//! The rmcp server handler exposing overgraph's tools.
//!
//! Six tools:
//!
//! - `graph_query` — run GQL: reads, writes (`CREATE`/`MERGE`/`SET`/`DELETE`),
//!   and schema DDL.
//! - `graph_upsert_node` — structured create-or-update of one node by
//!   `(label, key)`; with `embed_text`, also stores a dense embedding of the
//!   given text on the node.
//! - `graph_upsert_edge` — structured edge creation between two resolved
//!   nodes, endpoint keys resolved up front.
//! - `graph_vector_search` — semantic nearest-neighbor search: the query text
//!   is embedded server-side and matched against stored embeddings.
//! - `graph_ingest_file` — bulk-load a JSONL batch file of nodes and edges
//!   in one call: every line validates, every edge endpoint resolves, and
//!   only then do chunked engine writes go out.
//! - `graph_docs` — the bundled GQL reference ([`crate::docs`]): the vendored
//!   syntax companion verbatim, whole or by `## ` section, without ever
//!   touching the engine.
//!
//! Engine failures are **tool-level** errors (`CallToolResult::error`): the
//! model submitted a runnable request and the engine rejected it, so the
//! reason belongs in content the model can read and react to. The same rule
//! covers embedding failures (the model could not load or the download
//! failed). Protocol-level errors (`ErrorData`) are reserved for requests
//! that cannot be routed at all — an unknown tool name, or arguments that
//! cannot be interpreted as the tool's input.

use std::collections::BTreeMap;
use std::sync::Arc;

use overgraph::types::{GqlExecutionOptions, GqlParams, UpsertEdgeOptions, UpsertNodeOptions};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use serde_json::Value;
use serde_json::json;
use std::fmt::Write as _;

use crate::convert;
use crate::docs;
use crate::embedder;
use crate::ingest;

/// The largest `k` a vector search may ask for; HNSW k is cheap but an
/// unbounded k on a big graph is a pointless result payload.
const MAX_VECTOR_SEARCH_K: u64 = 100;

/// The default result count for `graph_vector_search`.
const DEFAULT_VECTOR_SEARCH_K: u64 = 10;

/// The overgraph MCP server: an open engine, the embedder behind the vector
/// tools, and the six-tool handler.
pub struct OvergraphMcpServer {
    db: Arc<overgraph::DatabaseEngine>,
    embedder: Arc<dyn embedder::Embedder>,
}

/// Converts a JSON schema literal into rmcp's `JsonObject` input-schema form.
///
/// Panics only on a non-object literal in this file, which would be a bug in
/// the literal itself; `from_value` keeps that failure out of runtime data.
fn to_schema(value: serde_json::Value) -> rmcp::model::JsonObject {
    match serde_json::from_value(value) {
        Ok(object) => object,
        Err(err) => {
            tracing::error!(%err, "static tool schema literal is not a JSON object");
            rmcp::model::JsonObject::new()
        }
    }
}

/// Why an edge endpoint could not be resolved.
enum EndpointFailure {
    /// The engine itself failed the lookup.
    Engine(overgraph::EngineError),
    /// No node carries this (label, key).
    Missing(String),
}

impl EndpointFailure {
    /// Renders the failure as tool-level error content naming the endpoint.
    fn into_tool_error(self) -> CallToolResponse {
        match self {
            Self::Engine(err) => {
                tracing::warn!(tool = "graph_upsert_edge", error = %err, "overgraph tool failed");
                Self::engine_message("graph_upsert_edge", &format!("{err}"))
            }
            Self::Missing(endpoint) => {
                Self::engine_message("graph_upsert_edge", &format!("node not found: {endpoint}"))
            }
        }
    }

    fn engine_message(tool: &'static str, message: &str) -> CallToolResponse {
        CallToolResult::error(vec![ContentBlock::text(format!(
            "{tool} failed: {message}"
        ))])
        .into()
    }
}

impl OvergraphMcpServer {
    /// Wraps an open engine and the embedder behind the vector tools.
    pub fn new(db: Arc<overgraph::DatabaseEngine>, embedder: Arc<dyn embedder::Embedder>) -> Self {
        Self { db, embedder }
    }

    fn graph_query_tool() -> Tool {
        Tool::new(
            "graph_query",
            "Run a GQL statement against the graph database. Covers reads \
             (MATCH/RETURN/WHERE/ORDER BY/LIMIT), writes (CREATE, MERGE, SET, \
             REMOVE, DELETE, DETACH DELETE), and schema DDL (ALTER, CHECK, SHOW) \
             including property-index DDL. Parameterize with $name placeholders \
             and pass values in `params`.",
            Arc::new(to_schema(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The GQL statement to execute."
                    },
                    "params": {
                        "type": "object",
                        "description": "Optional named parameter values for \
                                       $name placeholders in the query.",
                        "additionalProperties": true
                    }
                },
                "required": ["query"]
            }))),
        )
    }

    fn graph_upsert_node_tool() -> Tool {
        Tool::new(
            "graph_upsert_node",
            "Create or update one node identified by (label, key). Existing \
             properties are replaced by `props` when provided. Prefer this over \
             graph_query for structured writes.",
            Arc::new(to_schema(json!({
                "type": "object",
                "properties": {
                    "label": {
                        "type": "string",
                        "description": "The node label (e.g. \"Person\")."
                    },
                    "key": {
                        "type": "string",
                        "description": "The node's unique key within the label."
                    },
                    "props": {
                        "type": "object",
                        "description": "Optional properties to set on the node.",
                        "additionalProperties": true
                    },
                    "weight": {
                        "type": "number",
                        "description": "Optional node weight (default 1.0)."
                    },
                    "embed_text": {
                        "type": "string",
                        "description": "Optional text to embed and store on the \
                                       node for semantic search. Provide exactly \
                                       the text whose meaning a future search \
                                       should find this node by; jinn embeds it \
                                       locally (bge-small-en-v1.5, 512-token \
                                       context, longer text is truncated). Omit \
                                       for nodes that should not surface in \
                                       graph_vector_search."
                    }
                },
                "required": ["label", "key"]
            }))),
        )
    }

    fn graph_upsert_edge_tool() -> Tool {
        Tool::new(
            "graph_upsert_edge",
            "Create an edge between two existing nodes, resolved by \
             (label, key). Both endpoints must already exist — upsert the nodes \
             first. Edge upserts target the (from, to, label) triple, so a \
             repeat call updates the existing edge's properties and weight.",
            Arc::new(to_schema(json!({
                "type": "object",
                "properties": {
                    "from_label": {
                        "type": "string",
                        "description": "Label of the source node."
                    },
                    "from_key": {
                        "type": "string",
                        "description": "Key of the source node."
                    },
                    "to_label": {
                        "type": "string",
                        "description": "Label of the target node."
                    },
                    "to_key": {
                        "type": "string",
                        "description": "Key of the target node."
                    },
                    "label": {
                        "type": "string",
                        "description": "The edge label (e.g. \"KNOWS\")."
                    },
                    "props": {
                        "type": "object",
                        "description": "Optional edge properties.",
                        "additionalProperties": true
                    },
                    "weight": {
                        "type": "number",
                        "description": "Optional edge weight (default 1.0)."
                    },
                    "valid_from": {
                        "type": "integer",
                        "description": "Optional validity-window start, epoch millis."
                    },
                    "valid_to": {
                        "type": "integer",
                        "description": "Optional validity-window end, epoch millis."
                    }
                },
                "required": ["from_label", "from_key", "to_label", "to_key", "label"]
            }))),
        )
    }

    /// `graph_vector_search`: semantic nearest-neighbor search by text.
    fn graph_vector_search_tool() -> Tool {
        Tool::new(
            "graph_vector_search",
            "Semantic search over nodes that carry an embedding (stored via \
             graph_upsert_node's embed_text). Embeds the query text locally \
             and returns the nearest nodes with a similarity score — use this \
             to find nodes by meaning when no exact key or property predicate \
             is known. Hits include the node's labels, key, and properties.",
            Arc::new(to_schema(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The text to search for by meaning."
                    },
                    "k": {
                        "type": "integer",
                        "description": "How many hits to return (default 10, \
                                       max 100)."
                    },
                    "labels": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional node-label filter: only nodes \
                                       carrying at least one of these labels \
                                       are searched."
                    }
                },
                "required": ["query"]
            }))),
        )
    }

    /// `graph_docs`: serve the bundled GQL reference, whole or by section.
    ///
    /// The default (no arguments, or `section: null`) is the whole document —
    /// the intended read. A `section` string serves just that `## ` section;
    /// it exists so a call truncated by an output limit can be recovered with
    /// one targeted re-read, not as a browsing API. This arm never touches the
    /// engine — there is no `self` to reach it with: docs serve even when no
    /// database is open.
    fn run_graph_docs(
        args: Option<&serde_json::Map<String, Value>>,
    ) -> Result<CallToolResponse, ErrorData> {
        let requested = match args.and_then(|args| args.get("section")) {
            None | Some(Value::Null) => None,
            Some(Value::String(section)) => Some(section.as_str()),
            Some(_) => {
                return Err(ErrorData::invalid_params(
                    "`section` must be a string",
                    None,
                ));
            }
        };

        let text = match requested {
            None => {
                format!("{}\n\n{}", docs::GQL_SUBSET_PROVENANCE, docs::GQL_SUBSET)
            }
            Some(title) => match docs::section(docs::GQL_SUBSET, title) {
                Some(found) => found,
                None => {
                    let titles: Vec<String> = docs::parse_sections(docs::GQL_SUBSET)
                        .into_iter()
                        .map(|(title, _)| title)
                        .collect();
                    let text = format!(
                        "no section titled `{title}`; valid sections: {}",
                        titles.join(", ")
                    );
                    tracing::warn!(tool = "graph_docs", message = %text, "overgraph tool failed");
                    return Ok(CallToolResult::error(vec![ContentBlock::text(text)]).into());
                }
            },
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
    }

    fn graph_docs_tool() -> Tool {
        let sections: Vec<String> = docs::parse_sections(docs::GQL_SUBSET)
            .into_iter()
            .map(|(title, _)| title)
            .collect();
        let description = format!(
            "Return the bundled GQL reference for overgraph's query language: \
             read syntax, the metadata-function vs property rule (e.g. use \
             weight(n), not n.weight), mutations, schema DDL, params, cursors, \
             and worked examples. Call with no arguments for the whole \
             document. If the result appears truncated by an output limit, \
             re-call with `section` set to one of: {}.",
            sections.join(", ")
        );
        Tool::new(
            "graph_docs",
            description,
            Arc::new(to_schema(json!({
                "type": "object",
                "properties": {
                    "section": {
                        "type": "string",
                        "description": "Optional: serve only this `## ` section \
                                       of the reference. Omit for the whole document."
                    }
                }
            }))),
        )
    }

    /// Shared argument extraction: this tool's arguments as a JSON object.
    ///
    /// Malformed arguments are a protocol error (`invalid_params`) — the
    /// request could not be interpreted as this tool's input at all.
    fn args(request: &CallToolRequestParams) -> Result<&serde_json::Map<String, Value>, ErrorData> {
        request
            .arguments
            .as_ref()
            .ok_or_else(|| ErrorData::invalid_params("missing tool arguments", None))
    }

    fn required_string<'a>(
        args: &'a serde_json::Map<String, Value>,
        field: &str,
    ) -> Result<&'a str, ErrorData> {
        args.get(field).and_then(Value::as_str).ok_or_else(|| {
            ErrorData::invalid_params(format!("missing string field `{field}`"), None)
        })
    }

    fn optional_props(
        args: &serde_json::Map<String, Value>,
    ) -> Result<Option<serde_json::Map<String, Value>>, ErrorData> {
        match args.get("props") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Object(map)) => Ok(Some(map.clone())),
            Some(_) => Err(ErrorData::invalid_params("`props` must be an object", None)),
        }
    }

    fn optional_weight(args: &serde_json::Map<String, Value>) -> Result<Option<f32>, ErrorData> {
        match args.get("weight") {
            None | Some(Value::Null) => Ok(None),
            Some(v @ Value::Number(_)) => v
                .as_f64()
                .map(|f| f as f32)
                .map(Some)
                .ok_or_else(|| ErrorData::invalid_params("`weight` must be a number", None)),
            Some(_) => Err(ErrorData::invalid_params("`weight` must be a number", None)),
        }
    }

    fn optional_epoch_millis(
        args: &serde_json::Map<String, Value>,
        field: &str,
    ) -> Result<Option<i64>, ErrorData> {
        match args.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_i64().map(Some).ok_or_else(|| {
                ErrorData::invalid_params(format!("`{field}` must be an integer"), None)
            }),
        }
    }

    fn optional_embed_text(
        args: &serde_json::Map<String, Value>,
    ) -> Result<Option<String>, ErrorData> {
        match args.get("embed_text") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => {
                if text.trim().is_empty() {
                    return Err(ErrorData::invalid_params(
                        "`embed_text` must not be empty",
                        None,
                    ));
                }
                Ok(Some(text.clone()))
            }
            Some(_) => Err(ErrorData::invalid_params(
                "`embed_text` must be a string",
                None,
            )),
        }
    }

    fn optional_integer(
        args: &serde_json::Map<String, Value>,
        field: &str,
    ) -> Result<Option<u64>, ErrorData> {
        match args.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_u64().map(Some).ok_or_else(|| {
                ErrorData::invalid_params(format!("`{field}` must be a positive integer"), None)
            }),
        }
    }

    fn optional_string_list(
        args: &serde_json::Map<String, Value>,
        field: &str,
    ) -> Result<Option<Vec<String>>, ErrorData> {
        match args.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(items)) => {
                let mut labels = Vec::with_capacity(items.len());
                for item in items {
                    match item.as_str() {
                        Some(label) => labels.push(label.to_owned()),
                        None => {
                            return Err(ErrorData::invalid_params(
                                format!("`{field}` must be an array of strings"),
                                None,
                            ));
                        }
                    }
                }
                // An explicitly empty list means "no filter", not "match
                // nothing" — the agent asked for no restriction.
                Ok(non_empty(labels))
            }
            Some(_) => Err(ErrorData::invalid_params(
                format!("`{field}` must be an array of strings"),
                None,
            )),
        }
    }

    /// Renders engine results into successful tool content.
    fn ok_json(value: &Value) -> CallToolResponse {
        CallToolResult::success(vec![ContentBlock::text(value.to_string())]).into()
    }

    /// Renders an engine failure as tool-level error content the model can read.
    fn engine_error(tool: &'static str, err: &overgraph::EngineError) -> CallToolResponse {
        tracing::warn!(tool, error = %err, "overgraph tool failed");
        CallToolResult::error(vec![ContentBlock::text(format!("{tool} failed: {err}"))]).into()
    }

    /// Executes one engine call, rendering engine failures as tool-level
    /// error content the model can read.
    fn engine_call(
        &self,
        tool: &'static str,
        call: impl FnOnce(&overgraph::DatabaseEngine) -> Result<Value, overgraph::EngineError>,
    ) -> CallToolResponse {
        match call(&self.db) {
            Ok(value) => Self::ok_json(&value),
            Err(err) => Self::engine_error(tool, &err),
        }
    }

    /// `graph_query`: execute GQL and serialize the result set.
    fn run_graph_query(
        &self,
        args: &serde_json::Map<String, Value>,
    ) -> Result<CallToolResponse, ErrorData> {
        let query = Self::required_string(args, "query")?;
        let params: GqlParams = match args.get("params") {
            None | Some(Value::Null) => GqlParams::new(),
            Some(Value::Object(map)) => {
                let mut converted = GqlParams::new();
                for (k, v) in map {
                    let pv = convert::json_to_gql_param_value(v)
                        .map_err(|e| ErrorData::invalid_params(format!("params.{k}: {e}"), None))?;
                    converted.insert(k.clone(), pv);
                }
                converted
            }
            Some(_) => {
                return Err(ErrorData::invalid_params(
                    "`params` must be an object",
                    None,
                ));
            }
        };

        // Default options: Auto mode, engine-default limits. Full scans stay
        // off by default — queries the planner cannot index return a legible
        // error the model can recover from by adding a key predicate.
        Ok(self.engine_call("graph_query", |db| {
            let result = db.execute_gql(query, &params, &GqlExecutionOptions::default())?;
            Ok(convert::gql_execution_result_to_json(&result))
        }))
    }

    /// `graph_upsert_node`: structured create-or-update of one node.
    ///
    /// Embedding is keyed on the presence of `embed_text`: present means
    /// embed-and-attach, absent means exactly the pure-graph v1 behavior.
    async fn run_graph_upsert_node(
        &self,
        args: &serde_json::Map<String, Value>,
    ) -> Result<CallToolResponse, ErrorData> {
        let label = Self::required_string(args, "label")?;
        let key = Self::required_string(args, "key")?;
        let props = Self::optional_props(args)?;
        let weight = Self::optional_weight(args)?;
        let embed_text = Self::optional_embed_text(args)?;

        let props = match props {
            None => BTreeMap::new(),
            Some(map) => convert::json_map_to_prop_map(&map).map_err(|(field, e)| {
                ErrorData::invalid_params(format!("props.{field}: {e}"), None)
            })?,
        };

        // The embed happens before the engine call so a failure leaves the
        // node untouched: no partial upsert that "succeeded" without its
        // vector. An embed failure is tool-level content naming the download.
        let dense_vector = match embed_text {
            None => None,
            Some(text) => match crate::embedder::embed_async(&self.embedder, &text).await {
                Ok(vector) => Some(vector),
                Err(report) => {
                    let message = format!(
                        "graph_upsert_node failed: {}",
                        report.current_context().message()
                    );
                    tracing::warn!(tool = "graph_upsert_node", message = %message, "overgraph tool failed");
                    return Ok(CallToolResult::error(vec![ContentBlock::text(message)]).into());
                }
            },
        };

        let options = UpsertNodeOptions {
            props,
            weight: weight.unwrap_or_default(),
            dense_vector,
            sparse_vector: None,
        };

        Ok(self.engine_call("graph_upsert_node", |db| {
            db.upsert_node(label, key, options)
                .map(|node_id| json!({ "node_id": node_id }))
        }))
    }

    /// `graph_vector_search`: embed the query, search, enrich the hits.
    async fn run_graph_vector_search(
        &self,
        args: &serde_json::Map<String, Value>,
    ) -> Result<CallToolResponse, ErrorData> {
        let query = Self::required_string(args, "query")?;
        let k = Self::optional_integer(args, "k")?
            .unwrap_or(DEFAULT_VECTOR_SEARCH_K)
            .clamp(1, MAX_VECTOR_SEARCH_K) as usize;
        let label_filter = Self::optional_string_list(args, "labels")?.map(|labels| {
            overgraph::types::NodeLabelFilter {
                labels,
                mode: overgraph::types::LabelMatchMode::Any,
            }
        });

        // Embed before touching the engine, off the async worker; the
        // failure path (cold model, failed download) is tool-level content.
        let dense_query = match crate::embedder::embed_async(&self.embedder, query).await {
            Ok(vector) => Some(vector),
            Err(report) => {
                let message = format!(
                    "graph_vector_search failed: {}",
                    report.current_context().message()
                );
                tracing::warn!(tool = "graph_vector_search", message = %message, "overgraph tool failed");
                return Ok(CallToolResult::error(vec![ContentBlock::text(message)]).into());
            }
        };

        let request = overgraph::types::VectorSearchRequest {
            mode: overgraph::types::VectorSearchMode::Dense,
            dense_query,
            sparse_query: None,
            k,
            label_filter,
            ef_search: None,
            scope: None,
            dense_weight: None,
            sparse_weight: None,
            fusion_mode: None,
        };

        let hits = match self.db.vector_search(&request) {
            Ok(hits) => hits,
            Err(err) => return Ok(Self::engine_error("graph_vector_search", &err)),
        };

        // Enrich each hit with the node's shape; a node deleted between the
        // search and this read is skipped, not an error — the hit list was
        // already committed by the search.
        let results: Vec<Value> = hits
            .iter()
            .filter_map(|hit| {
                let node = self.db.get_node(hit.node_id).ok().flatten()?;
                Some(json!({
                    "node_id": hit.node_id,
                    "score": hit.score,
                    "labels": node.labels,
                    "key": node.key,
                    "props": convert::prop_map_to_json(&node.props),
                }))
            })
            .collect();

        Ok(Self::ok_json(&json!({
            "query": query,
            "hits": results,
        })))
    }

    /// `graph_upsert_edge`: resolve both endpoints, then create the edge.
    fn run_graph_upsert_edge(
        &self,
        args: &serde_json::Map<String, Value>,
    ) -> Result<CallToolResponse, ErrorData> {
        let from_label = Self::required_string(args, "from_label")?;
        let from_key = Self::required_string(args, "from_key")?;
        let to_label = Self::required_string(args, "to_label")?;
        let to_key = Self::required_string(args, "to_key")?;
        let label = Self::required_string(args, "label")?;
        let props = Self::optional_props(args)?;
        let weight = Self::optional_weight(args)?;
        let valid_from = Self::optional_epoch_millis(args, "valid_from")?;
        let valid_to = Self::optional_epoch_millis(args, "valid_to")?;

        let props = match props {
            None => BTreeMap::new(),
            Some(map) => convert::json_map_to_prop_map(&map).map_err(|(field, e)| {
                ErrorData::invalid_params(format!("props.{field}: {e}"), None)
            })?,
        };

        // Endpoint resolution happens here, not inside the engine: a missing
        // endpoint is a caller mistake worth naming, never an auto-create.
        let resolve_endpoint = |node_label: &str, node_key: &str| -> Result<u64, EndpointFailure> {
            match self.db.get_node_by_key(node_label, node_key) {
                Err(err) => Err(EndpointFailure::Engine(err)),
                Ok(None) => Err(EndpointFailure::Missing(format!("{node_label}:{node_key}"))),
                Ok(Some(node)) => Ok(node.id),
            }
        };
        let from = resolve_endpoint(from_label, from_key);
        let to = resolve_endpoint(to_label, to_key);
        let (from, to) = match (from, to) {
            (Ok(from), Ok(to)) => (from, to),
            (Err(failure), _) | (_, Err(failure)) => {
                return Ok(failure.into_tool_error());
            }
        };

        let options = UpsertEdgeOptions {
            props,
            weight: weight.unwrap_or_default(),
            valid_from,
            valid_to,
        };

        Ok(self.engine_call("graph_upsert_edge", |db| {
            db.upsert_edge(from, to, label, options)
                .map(|edge_id| json!({ "edge_id": edge_id }))
        }))
    }

    /// `graph_ingest_file`: ingest a JSONL batch file in one tool call.
    ///
    /// The three-stage pipeline in [`crate::ingest`] carries the guarantees:
    /// every line validates before anything else happens, every edge
    /// endpoint resolves before the first write (so a missing endpoint
    /// aborts the whole file with zero writes), and the writes go out in
    /// engine-sized chunks — each a real commit, so a mid-write failure
    /// reports what landed and a re-run finishes idempotently.
    async fn run_graph_ingest_file(
        &self,
        args: &serde_json::Map<String, Value>,
    ) -> Result<CallToolResponse, ErrorData> {
        let path = Self::required_string(args, "path")?;
        let batch_size = Self::optional_integer(args, "batch_size")?
            .unwrap_or(ingest::DEFAULT_INGEST_BATCH_SIZE);
        if batch_size == 0 || batch_size > ingest::MAX_INGEST_BATCH_SIZE {
            return Err(ErrorData::invalid_params(
                format!(
                    "`batch_size` must be between 1 and {}",
                    ingest::MAX_INGEST_BATCH_SIZE
                ),
                None,
            ));
        }

        let batch = match ingest::load_batch_file(std::path::Path::new(path)) {
            Ok(batch) => batch,
            Err(err) => {
                let message = format!("graph_ingest_file failed: {}", err.message());
                tracing::warn!(tool = "graph_ingest_file", message = %message, "overgraph tool failed");
                return Ok(CallToolResult::error(vec![ContentBlock::text(message)]).into());
            }
        };

        // Endpoint pre-validation runs before the first write, so the
        // missing-endpoint abort leaves the graph untouched.
        let missing = match ingest::missing_endpoints(&batch, &self.db) {
            Ok(missing) => missing,
            Err(err) => return Ok(Self::engine_error("graph_ingest_file", &err)),
        };
        if !missing.is_empty() {
            return Ok(Self::ingest_abort(
                &missing,
                batch.nodes.len(),
                batch.edges.len(),
            ));
        }

        let file_name = std::path::Path::new(path).file_name().map_or_else(
            || path.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let started = std::time::Instant::now();
        let outcome = match ingest::ingest_batch_file(
            batch,
            &self.db,
            &self.embedder,
            batch_size as usize,
            &file_name,
            &|line| crate::embedder::announce(line),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(err) => {
                let message = format!("graph_ingest_file failed: {}", err.message());
                tracing::warn!(tool = "graph_ingest_file", message = %message, "overgraph tool failed");
                return Ok(CallToolResult::error(vec![ContentBlock::text(message)]).into());
            }
        };

        Ok(Self::ok_json(&json!({
            "nodes_upserted": outcome.nodes_upserted,
            "edges_upserted": outcome.edges_upserted,
            "embedded": outcome.embedded,
            "engine_batches": outcome.engine_batches,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        })))
    }

    /// Renders the missing-endpoint abort as tool-level error content.
    ///
    /// Lists up to [`ingest::MAX_LISTED_MISSING_ENDPOINTS`] offenders with
    /// their first referencing line, says nothing was written, and points at
    /// the fix (add the node lines).
    fn ingest_abort(
        missing: &[ingest::MissingEndpoint],
        node_count: usize,
        edge_count: usize,
    ) -> CallToolResponse {
        let mut message = format!(
            "graph_ingest_file failed: {} edge endpoint(s) resolve to no node \
             — nothing was written ({} node line(s) and {} edge line(s) were \
             validated but discarded). Add node lines for:",
            missing.len(),
            node_count,
            edge_count,
        );
        let listed = missing.len().min(ingest::MAX_LISTED_MISSING_ENDPOINTS);
        for endpoint in missing.iter().take(listed) {
            let _ = writeln!(
                message,
                "\n  - {} (first referenced on line {})",
                endpoint.endpoint, endpoint.first_line
            );
        }
        if missing.len() > listed {
            let _ = write!(message, "\n  - and {} more", missing.len() - listed);
        }
        tracing::warn!(tool = "graph_ingest_file", message = %message, "overgraph tool failed");
        CallToolResult::error(vec![ContentBlock::text(message)]).into()
    }

    /// `graph_ingest_file`'s schema: the file contract in the description.
    fn graph_ingest_file_tool() -> Tool {
        Tool::new(
            "graph_ingest_file",
            "Bulk-load a batch file of nodes and edges in one call — for a \
             corpus, generate a JSONL file with a script (one object per \
             line, UTF-8) and ingest it here instead of calling \
             graph_upsert_node/graph_upsert_edge per record. File contract: \
             each line carries exactly one of `\"node\": {...}` or \
             `\"edge\": {...}`. Node line: {\"label\": str, \"key\": str, \
             \"props\"?: obj, \"weight\"?: num, \"embed_text\"?: str} — \
             embed_text is embedded locally like graph_upsert_node's. Edge \
             line: {\"label\": str, \"from\": \"Label:key\", \"to\": \
             \"Label:key\", \"props\"?: obj, \"weight\"?: num, \
             \"valid_from\"?: epoch-ms, \"valid_to\"?: epoch-ms}. Endpoints \
             are `Label:key` strings (split at the first colon, so keys may \
             contain colons). Edges may reference nodes declared ANYWHERE in \
             the file (even on later lines) or already in the database — all \
             nodes are upserted before any edge is resolved, so line order \
             never matters. If any edge endpoint resolves to nothing, the \
             whole file aborts with zero writes and the missing endpoints are \
             named with their line numbers. Upserts are idempotent by \
             (label, key): a failed or interrupted run can re-run the same \
             file safely. `batch_size` caps each engine write batch (default \
             512, max 4096); progress goes to the server's stderr. The \
             response carries counts and timing only — resolve ids later via \
             graph_query.",
            Arc::new(to_schema(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path of the JSONL batch file to \
                                       ingest, readable by the server \
                                       process."
                    },
                    "batch_size": {
                        "type": "integer",
                        "description": "Optional nodes/edges per engine \
                                       write batch (default 512, max 4096)."
                    }
                },
                "required": ["path"]
            }))),
        )
    }
}

impl ServerHandler for OvergraphMcpServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }

    // Trait-required async signature; listing is static and never awaits.
    #[expect(clippy::unused_async_trait_impl, reason = "trait signature is async")]
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![
            Self::graph_query_tool(),
            Self::graph_upsert_node_tool(),
            Self::graph_upsert_edge_tool(),
            Self::graph_vector_search_tool(),
            Self::graph_ingest_file_tool(),
            Self::graph_docs_tool(),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // Unknown tools cannot be routed at all — a protocol error, not a
        // tool-level one.
        match request.name.as_ref() {
            "graph_query" => self.run_graph_query(Self::args(&request)?),
            "graph_upsert_node" => self.run_graph_upsert_node(Self::args(&request)?).await,
            "graph_upsert_edge" => self.run_graph_upsert_edge(Self::args(&request)?),
            "graph_vector_search" => self.run_graph_vector_search(Self::args(&request)?).await,
            "graph_ingest_file" => self.run_graph_ingest_file(Self::args(&request)?).await,
            // Docs serve without arguments at all, so this arm reads the
            // optional map directly instead of going through `Self::args`.
            "graph_docs" => Self::run_graph_docs(request.arguments.as_ref()),
            other => Err(ErrorData::invalid_params(
                format!(
                    "unknown tool `{other}`; expected one of: graph_query, \
                     graph_upsert_node, graph_upsert_edge, graph_vector_search, \
                     graph_ingest_file, graph_docs"
                ),
                None,
            )),
        }
    }
}

/// Drops an empty list, mapping it to `None` (no restriction).
fn non_empty(labels: Vec<String>) -> Option<Vec<String>> {
    if labels.is_empty() {
        None
    } else {
        Some(labels)
    }
}
