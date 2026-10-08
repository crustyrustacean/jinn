//! The bundled graphs MCP server — shared, durable, graph-shaped state.
//!
//! One streamable-HTTP MCP server (mounted on axum at `/mcp`) over one
//! minigraf database directory. Configured in `jinn.toml` under `[mcp.graphs]`
//! with `singleton = true`, so jinn spawns exactly one server process for the
//! whole app and every session that enables the server reads and writes the
//! same graphs through it.
//!
//! Four tools, all namespaced `mcp__graphs__*`:
//!
//! - [`create_graph`](tools::CreateGraphTool) — create `<name>.graph`; fails
//!   if it already exists, leaving the file untouched.
//! - [`transact`](tools::TransactTool) — run one Datalog command
//!   (`(transact ...)`, `(retract ...)`, `(rule ...)`); returns the
//!   transaction id.
//! - [`query`](tools::QueryTool) — run one `(query ...)` including
//!   `:as-of`/`:valid-at` time travel; typed rows come back as JSON.
//! - [`list_graphs`](tools::ListGraphsTool) — names of the graphs on disk.
//!
//! # Handle discipline
//!
//! One [`Minigraf`] handle per graph per process, held for the server's life
//! in a name-keyed cache. Nothing re-opens a path: minigraf refuses a second
//! in-process open of one file (`STG-025`), so a per-call open design would
//! fail on the second call. Cross-process contention is minigraf's own lock
//! refusal (`STG-026`) and surfaces as a tool error — never corruption, never
//! a hang.
//!
//! # Thread discipline
//!
//! Every minigraf call sits in [`tokio::task::spawn_blocking`]: `execute` is
//! synchronous, and a `WriteTransaction` is `!Send`. The server deliberately
//! uses auto-commit `execute` per tool call — each call is already an atomic
//! WAL transaction — and never holds an explicit transaction across an await.
//!
//! # Create-before-use
//!
//! Only `create_graph` creates a `.graph` file. `transact`/`query` on an
//! unknown graph name are refused with an error naming `create_graph` as the
//! remedy. The cache-miss open path checks existence first, because a plain
//! minigraf open would create the file.

use std::path::PathBuf;
use std::sync::Arc;

use error_stack::{Report, ResultExt};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorData, Implementation, InitializeResult, ServerCapabilities,
};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

pub mod errors;
pub mod json_rows;
pub mod naming;
pub mod store;

use store::GraphStore;

/// The `ServerHandler` served at `/mcp`: the four graph tools over one
/// [`GraphStore`]. The tool router is the associated function the
/// `#[tool_router]`/`#[tool_handler]` pair generates (`Self::tool_router()`).
#[derive(Clone)]
pub struct GraphsServer {
    store: Arc<GraphStore>,
}

/// Request payload for the `create_graph` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateGraphArgs {
    /// Filesystem-safe graph name (`[a-z0-9_-]+`).
    pub name: String,
}

/// Request payload for the `transact` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TransactArgs {
    /// Target graph name; must exist (create it with `create_graph`).
    pub graph: String,
    /// One Datalog write command: `(transact ...)`, `(retract ...)`, or `(rule ...)`.
    pub datalog: String,
}

/// Request payload for the `query` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryArgs {
    /// Target graph name; must exist (create it with `create_graph`).
    pub graph: String,
    /// One Datalog `(query ...)` command; `:as-of`/`:valid-at` time travel works.
    pub datalog: String,
}

#[tool_router]
impl GraphsServer {
    /// Builds the server over `graphs_dir`, creating the directory if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the graphs directory cannot be created.
    pub fn new(graphs_dir: PathBuf) -> Result<Self, Report<errors::GraphsError>> {
        let server = Self {
            store: Arc::new(GraphStore::new(graphs_dir)),
        };
        server.store.ensure_dir()?;
        Ok(server)
    }

    #[tool(
        description = "Create a new graph database. The name must be filesystem-safe: lowercase letters, digits, hyphen, underscore. Fails if a graph with this name already exists — the existing graph is left untouched. Graphs persist across restarts; there is no delete tool by design."
    )]
    async fn create_graph(
        &self,
        Parameters(args): Parameters<CreateGraphArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = args.name;
        let store = self.store.clone();
        // minigraf open/create is synchronous file I/O: off the async runtime.
        let outcome =
            tokio::task::spawn_blocking(move || store.create_graph(&name).map(|()| name.clone()))
                .await
                .map_err(|e| errors::tool_error(&format!("create task failed: {e}")))?;
        match outcome {
            Ok(name) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "created graph `{name}`"
            ))])),
            Err(report) => Err(errors::tool_error(&format!("{report:?}"))),
        }
    }

    #[tool(
        description = "Run one Datalog write command against an existing graph: `(transact [[:entity :attr value] ...])` to assert facts, `(retract [[:entity :attr value]])` to remove, or `(rule [(name ?a ?b) ...])` to register a rule. Rules are session-only: they live in memory and are gone after a restart — re-register them after any restart. SAFETY: one value per (entity, attribute) per transact — two values for the same attribute in one call can read back as one (minigraf #371); split multi-valued attributes across separate transact calls. Keep facts small (well under 4 KB): store file paths in facts, not payloads. The graph must exist — create it with create_graph first. Returns the transaction id."
    )]
    async fn transact(
        &self,
        Parameters(args): Parameters<TransactArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let graph = args.graph;
        let datalog = args.datalog;
        let store = self.store.clone();
        let outcome = tokio::task::spawn_blocking(move || store.transact(&graph, &datalog))
            .await
            .map_err(|e| errors::tool_error(&format!("transact task failed: {e}")))?;
        match outcome {
            Ok(tx_id) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "committed transaction {tx_id}"
            ))])),
            Err(report) => Err(errors::tool_error(&format!("{report:?}"))),
        }
    }

    #[tool(
        description = "Run one Datalog query against an existing graph: `(query [:find ?a ?b :where [?e :attr ?a] ...])`. Supports bi-temporal time travel with `:as-of <tx-count>` (transaction time) and `:valid-at <iso-8601 | :any-valid-time>` (valid time), so history and audit questions are ordinary queries. Rows come back as JSON arrays aligned with the :find variables. The graph must exist — create it with create_graph first."
    )]
    async fn query(
        &self,
        Parameters(args): Parameters<QueryArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let graph = args.graph;
        let datalog = args.datalog;
        let store = self.store.clone();
        let outcome = tokio::task::spawn_blocking(move || store.query(&graph, &datalog))
            .await
            .map_err(|e| errors::tool_error(&format!("query task failed: {e}")))?;
        match outcome {
            Ok(json) => Ok(CallToolResult::success(vec![ContentBlock::text(json)])),
            Err(report) => Err(errors::tool_error(&format!("{report:?}"))),
        }
    }

    #[tool(
        description = "List every graph in the graphs directory (names only, sorted). Use a listed name with transact/query; create a missing graph with create_graph."
    )]
    async fn list_graphs(&self) -> Result<CallToolResult, ErrorData> {
        let store = self.store.clone();
        let outcome = tokio::task::spawn_blocking(move || store.list_graphs())
            .await
            .map_err(|e| errors::tool_error(&format!("list task failed: {e}")))?;
        match outcome {
            Ok(names) => {
                let body = if names.is_empty() {
                    "no graphs yet — create one with create_graph".to_owned()
                } else {
                    names.join("\n")
                };
                Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
            }
            Err(report) => Err(errors::tool_error(&format!("{report:?}"))),
        }
    }
}

#[tool_handler]
#[expect(
    clippy::unused_async_trait_impl,
    reason = "rmcp's tool_handler generates the async ServerHandler methods; none await"
)]
impl ServerHandler for GraphsServer {
    fn get_info(&self) -> InitializeResult {
        // `InitializeResult`/`ServerCapabilities` are `#[non_exhaustive]`:
        // build through `new` + the `with_*` setters, and enable tools
        // through the capabilities builder.
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities)
            .with_server_info(Implementation::new(
                "jinn-graphs",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Bi-temporal graph state for jinn sessions. create_graph before first use of \
                 a name; transact writes facts; query reads them, including :as-of/:valid-at \
                 history. One value per (entity, attribute) per transact.",
            )
    }
}

/// Runs the server until cancelled: binds `host:port`, serves MCP at `/mcp`,
/// and returns when the process is killed or the listener fails.
///
/// The `/mcp` route is mounted before the listener reports readiness from
/// the caller's perspective — a jinn MCP client retries connection-refused
/// indefinitely, but a server that binds and then serves slowly would wedge
/// the session reconcile, so the router is fully built before `serve`.
///
/// # Errors
///
/// Returns an error if the graphs directory cannot be created or the
/// listener cannot bind.
pub async fn run(
    host: &str,
    port: u16,
    graphs_dir: PathBuf,
) -> Result<(), Report<errors::GraphsError>> {
    let server = GraphsServer::new(graphs_dir)?;
    let service: rmcp::transport::streamable_http_server::StreamableHttpService<
        GraphsServer,
        rmcp::transport::streamable_http_server::session::local::LocalSessionManager,
    > = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        ),
        rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind((host, port))
        .await
        .change_context(errors::GraphsError::StoreDir(format!(
            "could not bind {host}:{port}"
        )))?;
    let bound = listener.local_addr().ok();
    tracing::info!(addr = ?bound, "jinn-graphs: serving MCP at /mcp");

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .change_context(errors::GraphsError::StoreDir(
            "serving /mcp failed".to_owned(),
        ))?;
    Ok(())
}
