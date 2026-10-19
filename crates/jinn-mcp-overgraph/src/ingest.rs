//! Batch-file ingestion behind `graph_ingest_file`.
//!
//! A batch file is self-describing JSONL: each line is a JSON object carrying
//! exactly one of a `node` or an `edge`, with edge endpoints referenced as
//! `Label:key` strings. The pipeline is three stages, each with its own
//! all-or-nothing guarantee:
//!
//! 1. [`load_batch_file`] / [`parse_batch_file`] — read and validate every
//!    line against the line schema; the first bad line fails the whole file,
//!    naming its 1-based line number, before any engine call happens.
//! 2. [`missing_endpoints`] — resolve every edge endpoint against the file's
//!    own nodes and the database; any miss aborts with zero writes.
//! 3. [`ingest_batch_file`] — upsert nodes in engine-sized chunks (embedding
//!    `embed_text` as it goes), then edges. Every chunk is a real commit, so
//!    a mid-write failure reports how much landed and a re-run finishes the
//!    job idempotently (upserts key on `(label, key)`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use overgraph::DatabaseEngine;
use overgraph::types::{EdgeInput, NodeInput, PropValue};
use wherror::Error;

use crate::convert;
use crate::embedder::embed_async;
use crate::embedder::{self, Embedder};

/// The `batch_size` used when the tool call omits it: big enough that a
/// 100k-line file stays dozens of engine calls, small enough that no single
/// WAL record dominates the write.
pub const DEFAULT_INGEST_BATCH_SIZE: u64 = 512;

/// The largest accepted `batch_size`; the hard cap bounds a single WAL
/// record's size no matter how big the file is.
pub const MAX_INGEST_BATCH_SIZE: u64 = 4096;

/// Weight applied to file lines that omit `weight` (the engine's own
/// default; `NodeInput`/`EdgeInput` have no `Default` of their own).
const DEFAULT_INGEST_WEIGHT: f32 = 1.0;

/// The most missing endpoints listed in an abort message before it
/// truncates with an "and N more" line.
pub const MAX_LISTED_MISSING_ENDPOINTS: usize = 20;

/// Why a batch file could not be read or parsed.
///
/// The message is the whole failure as it should appear in tool content —
/// it names the file or the offending line number.
#[derive(Debug, Clone, Error)]
#[error(debug)]
pub struct IngestFileError {
    reason: String,
}

impl IngestFileError {
    /// Wraps a failure with its user-facing reason.
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// The failure text as it should appear in tool content.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.reason
    }
}

/// Why a mid-write ingest failed, carrying what already landed.
///
/// Chunks commit as they go, so a failure partway through leaves earlier
/// chunks durable. The message reports the landed counts and points at the
/// idempotent re-run; the accessor methods let the caller render the same
/// facts structurally if it ever needs to.
#[derive(Debug, Clone, Error)]
#[error(debug)]
pub struct IngestWriteError {
    reason: String,
    nodes_upserted: u64,
    edges_upserted: u64,
}

impl IngestWriteError {
    /// Builds the error from what landed before the failure and why.
    fn partial(nodes_upserted: u64, edges_upserted: u64, reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            nodes_upserted,
            edges_upserted,
        }
    }

    /// The failure text as it should appear in tool content: what landed,
    /// what went wrong, and why a re-run is safe.
    #[must_use]
    pub fn message(&self) -> String {
        format!(
            " write failed after {} node(s) and {} edge(s) upserted: {}; \
             upserts are idempotent by (label, key), so the same file can \
             safely be re-run",
            self.nodes_upserted, self.edges_upserted, self.reason
        )
    }

    /// Nodes that landed in committed chunks before the failure.
    #[must_use]
    pub fn nodes_upserted(&self) -> u64 {
        self.nodes_upserted
    }

    /// Edges that landed in committed chunks before the failure.
    #[must_use]
    pub fn edges_upserted(&self) -> u64 {
        self.edges_upserted
    }
}

/// A `Label:key` reference to a node, as written in an edge line's `from`/`to`.
///
/// The reference splits on the **first** colon, so a key may itself contain
/// colons; a label may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointRef {
    /// The node label (everything before the first colon).
    pub label: String,
    /// The node key within the label (everything after the first colon).
    pub key: String,
}

/// One node line of a batch file, validated and props-converted.
#[derive(Debug, Clone)]
pub struct BatchNode {
    /// The line's 1-based number in the file, for error messages.
    pub line: usize,
    /// The node's label.
    pub label: String,
    /// The node's unique key within the label.
    pub key: String,
    /// The node's properties (empty when the line omitted `props`).
    pub props: BTreeMap<String, PropValue>,
    /// The node's weight; `None` means the engine default applies.
    pub weight: Option<f32>,
    /// Text to embed onto the node; `None` leaves it out of vector search.
    pub embed_text: Option<String>,
}

/// One edge line of a batch file, validated and props-converted.
#[derive(Debug, Clone)]
pub struct BatchEdge {
    /// The line's 1-based number in the file, for error messages.
    pub line: usize,
    /// The edge label (e.g. `KNOWS`).
    pub label: String,
    /// The source endpoint.
    pub from: EndpointRef,
    /// The target endpoint.
    pub to: EndpointRef,
    /// The edge's properties (empty when the line omitted `props`).
    pub props: BTreeMap<String, PropValue>,
    /// The edge's weight; `None` means the engine default applies.
    pub weight: Option<f32>,
    /// Optional validity-window start, epoch millis.
    pub valid_from: Option<i64>,
    /// Optional validity-window end, epoch millis.
    pub valid_to: Option<i64>,
}

/// A fully parsed batch file: its nodes and edges, in file order.
#[derive(Debug, Default)]
pub struct BatchFile {
    /// Every `node` line, in file order.
    pub nodes: Vec<BatchNode>,
    /// Every `edge` line, in file order.
    pub edges: Vec<BatchEdge>,
}

/// An endpoint that resolved to no node during pre-validation.
#[derive(Debug, Clone)]
pub struct MissingEndpoint {
    /// The unresolvable reference, rendered `Label:key`.
    pub endpoint: String,
    /// The first edge line that referenced it.
    pub first_line: usize,
}

/// What a completed ingest did: counts and batching facts only, never ids.
#[derive(Debug, Clone, Copy)]
pub struct IngestOutcome {
    /// Nodes upserted (committed chunks).
    pub nodes_upserted: u64,
    /// Edges upserted (committed chunks).
    pub edges_upserted: u64,
    /// Nodes whose `embed_text` was embedded.
    pub embedded: u64,
    /// Engine write batches consumed (node chunks + edge chunks).
    pub engine_batches: u64,
}

/// Chunk-write facts for the node pass (private to the write path).
struct NodeWriteStats {
    upserted: u64,
    embedded: u64,
    batches: u64,
    /// `(label, key)` → engine node id for every node this pass wrote.
    key_to_id: HashMap<(String, String), u64>,
}

/// Chunk-write facts for the edge pass (private to the write path).
struct EdgeWriteStats {
    upserted: u64,
    batches: u64,
}

/// One parsed line, before it is filed under nodes or edges.
enum BatchLine {
    /// The line carried a `node`.
    Node(BatchNode),
    /// The line carried an `edge`.
    Edge(BatchEdge),
}

/// Reads and parses the batch file at `path`.
///
/// # Errors
///
/// Returns an error naming the path if the file cannot be read; an empty
/// file parses successfully into a zero-count [`BatchFile`].
pub fn load_batch_file(path: &Path) -> Result<BatchFile, IngestFileError> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| IngestFileError::new(format!("could not read {}: {err}", path.display())))?;
    parse_batch_file(&content)
}

/// Parses batch-file content: one JSON object per line, `node` or `edge`.
///
/// Blank lines are skipped (generators format freely); line numbers in
/// errors always count real lines. Parsing is fail-fast: the first invalid
/// line fails the file and nothing is returned, so a caller can never write
/// a file it did not fully validate.
///
/// # Errors
///
/// Returns an error naming the offending line for invalid JSON, a line
/// carrying both or neither of `node`/`edge`, or any field-level schema
/// violation (missing label/key, non-object props, empty `embed_text`,
/// malformed `Label:key` endpoint, non-finite weight).
pub fn parse_batch_file(content: &str) -> Result<BatchFile, IngestFileError> {
    let mut batch = BatchFile::default();
    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        match parse_line(line_number, line)? {
            BatchLine::Node(node) => batch.nodes.push(node),
            BatchLine::Edge(edge) => batch.edges.push(edge),
        }
    }
    Ok(batch)
}

/// Parses one line into its node or edge shape.
fn parse_line(line_number: usize, line: &str) -> Result<BatchLine, IngestFileError> {
    let value: serde_json::Value = serde_json::from_str(line)
        .map_err(|err| IngestFileError::new(format!("line {line_number}: invalid JSON: {err}")))?;
    let object = value.as_object().ok_or_else(|| {
        IngestFileError::new(format!("line {line_number}: must be a JSON object"))
    })?;
    let has_node = object.contains_key("node");
    let has_edge = object.contains_key("edge");
    match (has_node, has_edge) {
        (true, false) => Ok(BatchLine::Node(parse_node_line(line_number, object)?)),
        (false, true) => Ok(BatchLine::Edge(parse_edge_line(line_number, object)?)),
        (false, false) => Err(IngestFileError::new(format!(
            "line {line_number}: carries neither `node` nor `edge`; every line \
             must be an object with exactly one of those keys"
        ))),
        (true, true) => Err(IngestFileError::new(format!(
            "line {line_number}: carries both `node` and `edge`; every line \
             must carry exactly one of them"
        ))),
    }
}

/// Parses the `node` body of a node line.
fn parse_node_line(
    line_number: usize,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<BatchNode, IngestFileError> {
    let body = required_body(object, "node", line_number)?;
    let label = required_non_empty_string(body, "label", line_number)?;
    let key = required_non_empty_string(body, "key", line_number)?;
    let props = optional_props(body, line_number)?;
    let weight = optional_weight(body, line_number)?;
    let embed_text = optional_embed_text(body, line_number)?;
    Ok(BatchNode {
        line: line_number,
        label: label.to_owned(),
        key: key.to_owned(),
        props,
        weight,
        embed_text,
    })
}

/// Parses the `edge` body of an edge line.
fn parse_edge_line(
    line_number: usize,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<BatchEdge, IngestFileError> {
    let body = required_body(object, "edge", line_number)?;
    let label = required_non_empty_string(body, "label", line_number)?;
    let from = parse_endpoint(body, "from", line_number)?;
    let to = parse_endpoint(body, "to", line_number)?;
    let props = optional_props(body, line_number)?;
    let weight = optional_weight(body, line_number)?;
    let valid_from = optional_epoch_millis(body, "valid_from", line_number)?;
    let valid_to = optional_epoch_millis(body, "valid_to", line_number)?;
    Ok(BatchEdge {
        line: line_number,
        label: label.to_owned(),
        from,
        to,
        props,
        weight,
        valid_from,
        valid_to,
    })
}

/// Reads a required wrapper object (`"node": {...}` / `"edge": {...}`).
fn required_body<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
    line_number: usize,
) -> Result<&'a serde_json::Map<String, serde_json::Value>, IngestFileError> {
    object
        .get(field)
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            IngestFileError::new(format!("line {line_number}: `{field}` must be an object"))
        })
}

/// Reads a required non-empty string field.
fn required_non_empty_string<'a>(
    body: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
    line_number: usize,
) -> Result<&'a str, IngestFileError> {
    let value = body
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            IngestFileError::new(format!("line {line_number}: `{field}` must be a string"))
        })?;
    if value.trim().is_empty() {
        return Err(IngestFileError::new(format!(
            "line {line_number}: `{field}` must not be empty"
        )));
    }
    Ok(value)
}

/// Reads an optional props object, converting it to the engine's prop map.
fn optional_props(
    body: &serde_json::Map<String, serde_json::Value>,
    line_number: usize,
) -> Result<BTreeMap<String, PropValue>, IngestFileError> {
    match body.get("props") {
        None | Some(serde_json::Value::Null) => Ok(BTreeMap::new()),
        Some(serde_json::Value::Object(map)) => {
            convert::json_map_to_prop_map(map).map_err(|(field, error)| {
                IngestFileError::new(format!("line {line_number}: props.{field}: {error}"))
            })
        }
        Some(_) => Err(IngestFileError::new(format!(
            "line {line_number}: `props` must be an object"
        ))),
    }
}

/// Reads an optional finite weight.
fn optional_weight(
    body: &serde_json::Map<String, serde_json::Value>,
    line_number: usize,
) -> Result<Option<f32>, IngestFileError> {
    match body.get("weight") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value @ serde_json::Value::Number(_)) => {
            let weight = value.as_f64().ok_or_else(|| {
                IngestFileError::new(format!("line {line_number}: `weight` must be a number"))
            })?;
            if !weight.is_finite() {
                return Err(IngestFileError::new(format!(
                    "line {line_number}: `weight` must be finite"
                )));
            }
            Ok(Some(weight as f32))
        }
        Some(_) => Err(IngestFileError::new(format!(
            "line {line_number}: `weight` must be a number"
        ))),
    }
}

/// Reads an optional non-empty `embed_text`.
fn optional_embed_text(
    body: &serde_json::Map<String, serde_json::Value>,
    line_number: usize,
) -> Result<Option<String>, IngestFileError> {
    match body.get("embed_text") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) => {
            if text.trim().is_empty() {
                return Err(IngestFileError::new(format!(
                    "line {line_number}: `embed_text` must not be empty"
                )));
            }
            Ok(Some(text.clone()))
        }
        Some(_) => Err(IngestFileError::new(format!(
            "line {line_number}: `embed_text` must be a string"
        ))),
    }
}

/// Reads an optional epoch-millis integer field.
fn optional_epoch_millis(
    body: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    line_number: usize,
) -> Result<Option<i64>, IngestFileError> {
    match body.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or_else(|| {
            IngestFileError::new(format!("line {line_number}: `{field}` must be an integer"))
        }),
    }
}

/// Reads a `Label:key` endpoint reference, splitting on the first colon.
///
/// The split point makes a colon-bearing key legal by construction; a label
/// can never contain a colon because everything after the first one is the
/// key.
fn parse_endpoint(
    body: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    line_number: usize,
) -> Result<EndpointRef, IngestFileError> {
    let raw = body
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            IngestFileError::new(format!(
                "line {line_number}: edge `{field}` must be a `Label:key` string"
            ))
        })?;
    let (label, key) = raw.split_once(':').ok_or_else(|| {
        IngestFileError::new(format!(
            "line {line_number}: edge `{field}` must be `Label:key`, got `{raw}`"
        ))
    })?;
    if label.is_empty() || key.is_empty() {
        return Err(IngestFileError::new(format!(
            "line {line_number}: edge `{field}` needs a non-empty label and key, \
             got `{raw}`"
        )));
    }
    Ok(EndpointRef {
        label: label.to_owned(),
        key: key.to_owned(),
    })
}

/// Resolves every edge endpoint before any write happens.
///
/// An endpoint is satisfied by a node declared anywhere in the same file or
/// by a node already in the database; anything else comes back as a
/// [`MissingEndpoint`] naming the reference and its first referencing line.
/// The caller aborts the whole file on any entry — because this runs before
/// the first write, the abort is genuinely zero-write.
///
/// # Errors
///
/// Returns the engine's error if an endpoint lookup itself fails.
pub fn missing_endpoints(
    batch: &BatchFile,
    db: &DatabaseEngine,
) -> Result<Vec<MissingEndpoint>, overgraph::EngineError> {
    let file_keys: HashSet<(String, String)> = batch
        .nodes
        .iter()
        .map(|node| (node.label.clone(), node.key.clone()))
        .collect();
    let endpoints = batch
        .edges
        .iter()
        .flat_map(|edge| [(&edge.from, edge.line), (&edge.to, edge.line)]);
    unique_missing(endpoints, &file_keys, db)
}

/// Deduplicates endpoint references and checks the ones the file cannot
/// satisfy against the database, preserving first-reference order.
fn unique_missing<'a>(
    endpoints: impl Iterator<Item = (&'a EndpointRef, usize)>,
    file_keys: &HashSet<(String, String)>,
    db: &DatabaseEngine,
) -> Result<Vec<MissingEndpoint>, overgraph::EngineError> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut missing = Vec::new();
    for (endpoint, line) in endpoints {
        if file_keys.contains(&(endpoint.label.clone(), endpoint.key.clone())) {
            continue;
        }
        let keyed = (endpoint.label.clone(), endpoint.key.clone());
        if !seen.insert(keyed) {
            continue;
        }
        if db
            .get_node_by_key(&endpoint.label, &endpoint.key)?
            .is_none()
        {
            missing.push(MissingEndpoint {
                endpoint: format!("{}:{}", endpoint.label, endpoint.key),
                first_line: line,
            });
        }
    }
    Ok(missing)
}

/// Writes a fully validated batch file to the engine in chunked batches.
///
/// Nodes go first (chunked, each chunk one [`DatabaseEngine::batch_upsert_nodes`]
/// call, `embed_text` embedded just before its chunk writes), then edges
/// (chunked, endpoints resolved from the node pass's key→id map or the
/// database). `progress` receives one stderr-bound line per chunk.
///
/// # Errors
///
/// Returns [`IngestWriteError`] naming what landed and why the write
/// stopped; committed chunks stay durable and a re-run is idempotent.
pub async fn ingest_batch_file(
    batch: BatchFile,
    db: &DatabaseEngine,
    embedder: &Arc<dyn Embedder>,
    batch_size: usize,
    file_name: &str,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> Result<IngestOutcome, IngestWriteError> {
    let nodes =
        write_all_nodes(&batch.nodes, db, embedder, batch_size, file_name, progress).await?;
    let edges = write_all_edges(
        &batch.edges,
        db,
        batch_size,
        file_name,
        progress,
        &nodes.key_to_id,
        nodes.upserted,
    )?;
    Ok(IngestOutcome {
        nodes_upserted: nodes.upserted,
        edges_upserted: edges.upserted,
        embedded: nodes.embedded,
        engine_batches: nodes.batches + edges.batches,
    })
}

/// Upserts every node in chunked batches, embedding as it goes.
async fn write_all_nodes(
    nodes: &[BatchNode],
    db: &DatabaseEngine,
    embedder: &Arc<dyn Embedder>,
    batch_size: usize,
    file_name: &str,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> Result<NodeWriteStats, IngestWriteError> {
    let total = nodes.len();
    let mut key_to_id = HashMap::with_capacity(total);
    let mut upserted = 0u64;
    let mut embedded_count = 0u64;
    let mut batches = 0u64;
    for chunk in nodes.chunks(batch_size) {
        let (inputs, chunk_embedded) =
            build_node_inputs(chunk, embedder).await.map_err(|report| {
                IngestWriteError::partial(
                    upserted,
                    0,
                    format!("embedding failed: {}", report.current_context().message()),
                )
            })?;
        let ids = db.batch_upsert_nodes(inputs).map_err(|err| {
            IngestWriteError::partial(upserted, 0, format!("engine write failed: {err}"))
        })?;
        record_node_ids(chunk, &ids, &mut key_to_id);
        upserted += ids.len() as u64;
        embedded_count += chunk_embedded;
        batches += 1;
        progress(&format!(
            "overgraph: ingest {file_name}: {upserted}/{total} nodes"
        ));
    }
    Ok(NodeWriteStats {
        upserted,
        embedded: embedded_count,
        batches,
        key_to_id,
    })
}

/// Builds engine inputs for one node chunk, embedding `embed_text` per node.
///
/// # Errors
///
/// Returns the embedder's error if any embedding fails; no engine call has
/// happened for this chunk yet.
async fn build_node_inputs(
    chunk: &[BatchNode],
    embedder: &Arc<dyn Embedder>,
) -> Result<(Vec<NodeInput>, u64), error_stack::Report<embedder::EmbedError>> {
    let mut inputs = Vec::with_capacity(chunk.len());
    let mut embedded_count = 0u64;
    for node in chunk {
        let dense_vector = match &node.embed_text {
            None => None,
            Some(text) => {
                embedded_count += 1;
                Some(embed_async(embedder, text).await?)
            }
        };
        inputs.push(NodeInput {
            labels: vec![node.label.clone()],
            key: node.key.clone(),
            props: node.props.clone(),
            weight: node.weight.unwrap_or(DEFAULT_INGEST_WEIGHT),
            dense_vector,
            sparse_vector: None,
        });
    }
    Ok((inputs, embedded_count))
}

/// Records a chunk's returned ids into the `(label, key)` → id map.
fn record_node_ids(
    chunk: &[BatchNode],
    ids: &[u64],
    key_to_id: &mut HashMap<(String, String), u64>,
) {
    for (node, id) in chunk.iter().zip(ids.iter()) {
        key_to_id.insert((node.label.clone(), node.key.clone()), *id);
    }
}

/// Upserts every edge in chunked batches, resolving endpoints as it goes.
///
/// Synchronous by construction: the engine calls and endpoint lookups never
/// await, so the chunk loop runs inline inside the caller's future.
fn write_all_edges(
    edges: &[BatchEdge],
    db: &DatabaseEngine,
    batch_size: usize,
    file_name: &str,
    progress: &(dyn Fn(&str) + Send + Sync),
    key_to_id: &HashMap<(String, String), u64>,
    nodes_upserted: u64,
) -> Result<EdgeWriteStats, IngestWriteError> {
    let total = edges.len();
    let mut upserted = 0u64;
    let mut batches = 0u64;
    for chunk in edges.chunks(batch_size) {
        let inputs = build_edge_inputs(chunk, db, key_to_id)
            .map_err(|reason| IngestWriteError::partial(nodes_upserted, upserted, reason))?;
        let ids = db.batch_upsert_edges(inputs).map_err(|err| {
            IngestWriteError::partial(
                nodes_upserted,
                upserted,
                format!("engine write failed: {err}"),
            )
        })?;
        upserted += ids.len() as u64;
        batches += 1;
        progress(&format!(
            "overgraph: ingest {file_name}: {upserted}/{total} edges"
        ));
    }
    Ok(EdgeWriteStats { upserted, batches })
}

/// Builds engine inputs for one edge chunk, resolving each endpoint.
///
/// Pre-validation already proved every endpoint resolvable, so an unresolved
/// endpoint here means the graph changed mid-ingest; that is a hard error
/// naming the edge's line, not an auto-create.
///
/// # Errors
///
/// Returns the failure reason for the first unresolvable endpoint.
fn build_edge_inputs(
    chunk: &[BatchEdge],
    db: &DatabaseEngine,
    key_to_id: &HashMap<(String, String), u64>,
) -> Result<Vec<EdgeInput>, String> {
    let mut inputs = Vec::with_capacity(chunk.len());
    for edge in chunk {
        let from = resolve_node_id(&edge.from, db, key_to_id).ok_or_else(|| {
            format!(
                "line {}: endpoint {}:{} no longer resolves",
                edge.line, edge.from.label, edge.from.key
            )
        })?;
        let to = resolve_node_id(&edge.to, db, key_to_id).ok_or_else(|| {
            format!(
                "line {}: endpoint {}:{} no longer resolves",
                edge.line, edge.to.label, edge.to.key
            )
        })?;
        inputs.push(EdgeInput {
            from,
            to,
            label: edge.label.clone(),
            props: edge.props.clone(),
            weight: edge.weight.unwrap_or(DEFAULT_INGEST_WEIGHT),
            valid_from: edge.valid_from,
            valid_to: edge.valid_to,
        });
    }
    Ok(inputs)
}

/// Resolves one endpoint to an engine node id: the node pass's map first,
/// then the database (a pre-existing node).
fn resolve_node_id(
    endpoint: &EndpointRef,
    db: &DatabaseEngine,
    key_to_id: &HashMap<(String, String), u64>,
) -> Option<u64> {
    if let Some(id) = key_to_id.get(&(endpoint.label.clone(), endpoint.key.clone())) {
        return Some(*id);
    }
    db.get_node_by_key(&endpoint.label, &endpoint.key)
        .ok()
        .flatten()
        .map(|node| node.id)
}

/// Tests for the pure and engine-touching parts of the ingest pipeline.
#[cfg(test)]
#[expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test code asserts with expect and indexes small results for clear failures"
)]
mod tests {
    use super::*;

    /// A two-line file: one bare node, one bare edge.
    const MINIMAL_FILE: &str = concat!(
        r#"{"node": {"label": "Person", "key": "alice"}}"#,
        "\n",
        r#"{"edge": {"label": "KNOWS", "from": "Person:alice", "to": "Person:bob"}}"#,
        "\n",
    );

    #[rstest::rstest]
    #[test]
    fn parse_collects_mixed_nodes_and_edges_in_any_order() {
        // Given a file whose edge lines precede the node lines they reference.
        let batch = parse_batch_file(concat!(
            r#"{"edge": {"label": "KNOWS", "from": "Person:alice", "to": "Person:bob"}}"#,
            "\n",
            r#"{"node": {"label": "Person", "key": "alice"}}"#,
            "\n",
            r#"{"node": {"label": "Person", "key": "bob"}}"#,
            "\n",
        ))
        .expect("parses");

        // Then all three lines are filed, edges independent of node order.
        assert_eq!(batch.nodes.len(), 2, "both nodes parsed");
        assert_eq!(batch.edges.len(), 1, "the edge parsed");
        assert_eq!(
            batch.nodes[0].line, 2,
            "line numbers are 1-based file lines"
        );
    }

    #[rstest::rstest]
    #[test]
    fn parse_rejects_a_line_carrying_both_node_and_edge() {
        // Given a line that carries both keys.
        let result = parse_batch_file(concat!(
            r#"{"node": {"label": "Person", "key": "alice"}, "edge": {"label": "KNOWS", "from": "Person:alice", "to": "Person:bob"}}"#,
            "\n",
        ));

        // Then the file fails naming the line and the ambiguity.
        let err = result.expect_err("both keys must fail");
        assert!(
            err.message().contains("line 1") && err.message().contains("both"),
            "error names line and both keys: {}",
            err.message()
        );
    }

    #[rstest::rstest]
    #[test]
    fn parse_rejects_a_line_carrying_neither_key() {
        // Given a line that is an object but carries neither wrapper key.
        let result = parse_batch_file(concat!(r#"{"label": "Person", "key": "alice"}"#, "\n",));

        // Then the file fails naming the line and the missing keys.
        let err = result.expect_err("neither key must fail");
        assert!(
            err.message().contains("line 1") && err.message().contains("neither"),
            "error names line and neither: {}",
            err.message()
        );
    }

    #[rstest::rstest]
    #[test]
    fn parse_reports_the_first_malformed_line_and_stops() {
        // Given a file whose second line is not JSON.
        let result = parse_batch_file(concat!(
            r#"{"node": {"label": "Person", "key": "alice"}}"#,
            "\n",
            "not json at all",
            "\n",
        ));

        // Then the failure names line 2.
        let err = result.expect_err("malformed line must fail");
        assert!(
            err.message().contains("line 2") && err.message().contains("invalid JSON"),
            "error names line 2 and invalid JSON: {}",
            err.message()
        );
    }

    #[rstest::rstest]
    #[test]
    fn parse_skips_blank_lines_but_counts_real_line_numbers() {
        // Given a file with a blank line before an invalid one.
        let result = parse_batch_file(concat!(
            r#"{"node": {"label": "Person", "key": "alice"}}"#,
            "\n",
            "\n",
            r#"{"node": {"key": "no-label"}}"#,
            "\n",
        ));

        // Then the blank line is skipped and the error names real line 3.
        let err = result.expect_err("missing label must fail");
        assert!(
            err.message().contains("line 3") && err.message().contains("`label`"),
            "error names real line 3: {}",
            err.message()
        );
    }

    #[rstest::rstest]
    #[test]
    fn parse_rejects_an_empty_embed_text_naming_the_line() {
        // Given a node line whose embed_text is whitespace only.
        let result = parse_batch_file(concat!(
            r#"{"node": {"label": "Doc", "key": "x", "embed_text": "   "}}"#,
            "\n",
        ));

        // Then the file fails with the same rule as the upsert tool.
        let err = result.expect_err("empty embed_text must fail");
        assert!(
            err.message().contains("line 1") && err.message().contains("`embed_text`"),
            "error names line and embed_text: {}",
            err.message()
        );
    }

    #[rstest::rstest]
    #[test]
    fn parse_rejects_a_non_finite_or_non_numeric_weight() {
        // Given an edge line whose weight is a string.
        let result = parse_batch_file(concat!(
            r#"{"edge": {"label": "KNOWS", "from": "Person:a", "to": "Person:b", "weight": "heavy"}}"#,
            "\n",
        ));

        // Then the file fails naming the weight field.
        let err = result.expect_err("string weight must fail");
        assert!(
            err.message().contains("line 1") && err.message().contains("`weight`"),
            "error names line and weight: {}",
            err.message()
        );
    }

    #[rstest::rstest]
    #[test]
    fn endpoint_references_split_on_the_first_colon() {
        // Given an edge whose key itself contains a colon.
        let batch = parse_batch_file(concat!(
            r#"{"node": {"label": "Doc", "key": "a:b"}}"#,
            "\n",
            r#"{"edge": {"label": "REL", "from": "Doc:a:b", "to": "Doc:c"}}"#,
            "\n",
        ))
        .expect("parses");

        // Then the label is everything before the first colon and the key
        // keeps the rest.
        let edge = batch.edges.first().expect("edge parsed");
        assert_eq!(edge.from.label, "Doc");
        assert_eq!(edge.from.key, "a:b");
    }

    #[rstest::rstest]
    #[test]
    fn endpoint_references_require_a_colon_and_nonempty_sides() {
        // Given edge lines with a colon-less, label-less, and key-less
        // reference respectively.
        for raw in [
            r#"{"edge": {"label": "R", "from": "NoColon", "to": "Doc:c"}}"#,
            r#"{"edge": {"label": "R", "from": ":key", "to": "Doc:c"}}"#,
            r#"{"edge": {"label": "R", "from": "Doc:", "to": "Doc:c"}}"#,
        ] {
            // When parsing each single-line file.
            let result = parse_batch_file(raw);

            // Then each fails naming the field.
            let err = result.expect_err("bad endpoint must fail");
            assert!(
                err.message().contains("`from`"),
                "bad reference names the field: {}",
                err.message()
            );
        }
    }

    #[rstest::rstest]
    #[test]
    fn props_convert_into_the_engine_map() {
        // Given a node line carrying scalar and array properties.
        let batch = parse_batch_file(concat!(
            r#"{"node": {"label": "Person", "key": "alice", "props": {"age": 30, "tags": ["a", "b"]}}}"#,
            "\n",
        ))
        .expect("parses");

        // Then both properties converted into the engine's map.
        let node = batch.nodes.first().expect("node parsed");
        assert_eq!(node.props.len(), 2);
    }

    #[rstest::rstest]
    #[test]
    fn minimal_file_parses_into_one_node_and_one_edge() {
        // Given the minimal two-line file.
        let batch = parse_batch_file(MINIMAL_FILE).expect("parses");

        // Then exactly one node and one edge came out.
        assert_eq!(batch.nodes.len(), 1);
        assert_eq!(batch.edges.len(), 1);
    }

    #[rstest::rstest]
    #[test]
    fn parse_treats_an_empty_file_as_a_zero_count_batch() {
        // Given an empty (or whitespace-only) file.
        let batch = parse_batch_file("\n\n").expect("parses");

        // Then the batch carries nothing and ingest has nothing to do.
        assert_eq!(batch.nodes.len(), 0);
        assert_eq!(batch.edges.len(), 0);
    }

    #[rstest::rstest]
    #[test]
    fn load_batch_file_names_the_path_when_it_cannot_read() {
        // Given a path no file exists at.
        let path = std::path::Path::new("/nonexistent/batch.jsonl");

        // When loading the file.
        let result = load_batch_file(path);

        // Then the error names the path.
        let err = result.expect_err("missing file must fail");
        assert!(
            err.message().contains("/nonexistent/batch.jsonl"),
            "error names the path: {}",
            err.message()
        );
    }

    #[rstest::rstest]
    #[test]
    fn missing_endpoints_flags_only_unresolvable_references() {
        // Given a file whose own nodes cover alice, a database that covers
        // the platform team, and two endpoints that exist nowhere (bob is
        // referenced but never declared, Ghost:nobody is pure fiction).
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let db =
            overgraph::DatabaseEngine::open(dir.path(), &overgraph::types::DbOptions::default())
                .expect("engine opens");
        db.upsert_node(
            "Team",
            "platform",
            overgraph::types::UpsertNodeOptions::default(),
        )
        .expect("seed node");
        let batch = parse_batch_file(concat!(
            r#"{"node": {"label": "Person", "key": "alice"}}"#,
            "\n",
            r#"{"edge": {"label": "KNOWS", "from": "Person:alice", "to": "Person:bob"}}"#,
            "\n",
            r#"{"edge": {"label": "JOINS", "from": "Person:alice", "to": "Team:platform"}}"#,
            "\n",
            r#"{"edge": {"label": "LIKES", "from": "Person:alice", "to": "Ghost:nobody"}}"#,
            "\n",
        ))
        .expect("parses");

        // When pre-validating every endpoint against the file and database.
        let missing = missing_endpoints(&batch, &db).expect("lookups succeed");

        // Then only the unresolvable references are flagged, in first-seen
        // order with their first referencing line.
        assert_eq!(
            missing.len(),
            2,
            "file-internal (alice) and db-backed (platform) resolve: {missing:?}"
        );
        assert_eq!(missing[0].endpoint, "Person:bob");
        assert_eq!(missing[0].first_line, 2);
        assert_eq!(missing[1].endpoint, "Ghost:nobody");
        assert_eq!(missing[1].first_line, 4);

        db.close().expect("engine closes");
    }

    #[rstest::rstest]
    #[test]
    fn missing_endpoints_deduplicates_repeated_references() {
        // Given a file where two edges reference the same missing node.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let db =
            overgraph::DatabaseEngine::open(dir.path(), &overgraph::types::DbOptions::default())
                .expect("engine opens");
        let batch = parse_batch_file(concat!(
            r#"{"node": {"label": "Person", "key": "alice"}}"#,
            "\n",
            r#"{"edge": {"label": "KNOWS", "from": "Person:alice", "to": "Ghost:nobody"}}"#,
            "\n",
            r#"{"edge": {"label": "KNOWS", "from": "Person:alice", "to": "Ghost:nobody"}}"#,
            "\n",
        ))
        .expect("parses");

        // When pre-validating.
        let missing = missing_endpoints(&batch, &db).expect("lookups succeed");

        // Then the reference is reported once, at its first line.
        assert_eq!(missing.len(), 1, "deduplicated: {missing:?}");
        assert_eq!(missing[0].first_line, 2);

        db.close().expect("engine closes");
    }
}
