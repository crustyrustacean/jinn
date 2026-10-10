//! The bundled minigraf MCP server — shared, durable, graph-shaped state.
//!
//! One streamable-HTTP MCP server (mounted on axum at `/mcp`) over one
//! minigraf database directory and one graph-template directory. Configured
//! in `jinn.toml` under `[mcp.minigraf]` with `singleton = true`, so jinn
//! spawns exactly one server process for the whole app and every session that
//! enables the server reads and writes the same graphs through it.
//!
//! Six tools, all namespaced `mcp__minigraf__*`:
//!
//! - [`create_graph`](tools docs below) — create a blank `<name>.graph` and
//!   make it the caller's active graph; fails if it already exists, leaving
//!   the file untouched.
//! - [`instantiate_graph`] — create `<name>.graph` from a template's
//!   `template.dl` seed script (seed + `(rules [...])` forms) and activate it.
//! - [`attach_graph`] — bind an existing graph as the caller's active graph.
//! - [`transact`] — run one Datalog command (`(transact ...)`,
//!   `(retract ...)`, `(rule ...)`); returns the transaction id.
//! - [`query`] — run one `(query ...)` including `:as-of`/`:valid-at` time
//!   travel; typed rows come back as JSON.
//! - [`list_templates`] — names of the templates available for instantiation.
//!
//! `transact`/`query` keep an optional wire-level `graph` argument for
//! hand-driven MCP clients (curl, `jinn mcp minigraf`); jinn's tool dispatch
//! injects it from the session's active graph and the advertised schema
//! omits it, so a model never addresses a graph without a session binding.
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
//! Only `create_graph` and `instantiate_graph` create `.graph` files.
//! `transact`/`query`/`attach_graph` on an unknown graph name are refused
//! with an error naming the remedy. The cache-miss open path checks
//! existence first, because a plain minigraf open would create the file.

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
pub mod templates;

use store::MinigrafStore;

/// The `ServerHandler` served at `/mcp`: the four graph tools over one
/// [`MinigrafStore`]. The tool router is the associated function the
/// `#[tool_router]`/`#[tool_handler]` pair generates (`Self::tool_router()`).
#[derive(Clone)]
pub struct MinigrafServer {
    store: Arc<MinigrafStore>,
}

/// Request payload for the `create_graph` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateGraphArgs {
    /// Filesystem-safe graph name (`[a-z0-9_-]+`).
    pub name: String,
    /// Creating session id. Injected by jinn's dispatch; left `None` when the
    /// server is driven by hand.
    #[serde(default)]
    pub created_by: Option<String>,
}

/// Request payload for the `instantiate_graph` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct InstantiateGraphArgs {
    /// Template to instantiate (`list_templates` lists the names).
    pub template: String,
    /// Filesystem-safe graph name (`[a-z0-9_-]+`) for the new instance.
    pub name: String,
    /// Creating session id. Injected by jinn's dispatch; left `None` when the
    /// server is driven by hand.
    #[serde(default)]
    pub created_by: Option<String>,
}

/// Request payload for the `attach_graph` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AttachGraphArgs {
    /// Existing graph to bind as the session's active graph.
    pub name: String,
    /// Attaching session id. Injected by jinn's dispatch; left `None` when
    /// the server is driven by hand.
    #[serde(default)]
    pub created_by: Option<String>,
}

/// Request payload for the `transact` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TransactArgs {
    /// One Datalog write command: `(transact ...)`, `(retract ...)`, or `(rule ...)`.
    pub datalog: String,
    /// Target graph name; must exist. Optional at the wire level for
    /// hand-driven clients — jinn's dispatch injects it from the session's
    /// active graph, and the advertised schema omits it.
    #[serde(default)]
    pub graph: Option<String>,
}

/// Request payload for the `query` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryArgs {
    /// One Datalog `(query ...)` command; `:as-of`/`:valid-at` time travel works.
    pub datalog: String,
    /// Target graph name; must exist. Optional at the wire level for
    /// hand-driven clients — jinn's dispatch injects it from the session's
    /// active graph, and the advertised schema omits it.
    #[serde(default)]
    pub graph: Option<String>,
}

#[tool_router]
impl MinigrafServer {
    /// Builds the server over `minigraf_dir` with templates under
    /// `templates_dir`, creating the graph directory if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the minigraf directory cannot be created.
    pub fn new(
        minigraf_dir: PathBuf,
        templates_dir: PathBuf,
    ) -> Result<Self, Report<errors::MinigrafServerError>> {
        let server = Self {
            store: Arc::new(MinigrafStore::new(minigraf_dir, templates_dir)),
        };
        server.store.ensure_dir()?;
        Ok(server)
    }

    /// Renders a provenance outcome as the tool's success text.
    fn activated_text(outcome: store::GraphProvenance) -> String {
        match outcome.template.as_deref() {
            Some(template) => format!(
                "activated graph `{}` (instantiated from template `{template}`, created by {})",
                outcome.name, outcome.created_by
            ),
            None => format!(
                "activated graph `{}` (blank, created by {})",
                outcome.name, outcome.created_by
            ),
        }
    }

    #[tool(
        description = "Create a new blank graph database and bind it as your active graph — subsequent transact/query calls hit it. The name must be filesystem-safe: lowercase letters, digits, hyphen, underscore. Fails if a graph with this name already exists — the existing graph is left untouched. Graphs persist across restarts; there is no delete tool by design. For a graph with pre-seeded state and rules, use instantiate_graph instead."
    )]
    async fn create_graph(
        &self,
        Parameters(args): Parameters<CreateGraphArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = args.name;
        let created_by = args.created_by.unwrap_or_else(|| "unknown".to_owned());
        let store = self.store.clone();
        // minigraf open/create is synchronous file I/O: off the async runtime.
        let outcome = tokio::task::spawn_blocking(move || {
            store
                .create_graph(&name, &created_by)
                .map(|p| (name.clone(), p))
        })
        .await
        .map_err(|e| errors::tool_error(&format!("create task failed: {e}")))?;
        match outcome {
            Ok((name, provenance)) => {
                Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                    "{}\nactive graph: `{name}`",
                    Self::activated_text(provenance)
                ))]))
            }
            Err(report) => Err(errors::tool_error(&format!("{report:?}"))),
        }
    }

    #[tool(
        description = "Create a new graph from a template and bind it as your active graph — subsequent transact/query calls hit it. Templates live under the config dir's graphs directory, one directory per template with a hand-editable template.dl script; list_templates lists the names. The script's seed forms run once (facts), its (rules [...]) forms register the graph's derived relations and are re-applied automatically whenever the graph is opened fresh (facts are never replayed). Fails if a graph with this name already exists — the existing file is left untouched."
    )]
    async fn instantiate_graph(
        &self,
        Parameters(args): Parameters<InstantiateGraphArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let InstantiateGraphArgs {
            template,
            name,
            created_by,
        } = args;
        let created_by = created_by.unwrap_or_else(|| "unknown".to_owned());
        let store = self.store.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            store
                .instantiate_graph(&template, &name, &created_by)
                .map(|p| (name.clone(), p))
        })
        .await
        .map_err(|e| errors::tool_error(&format!("instantiate task failed: {e}")))?;
        match outcome {
            Ok((name, provenance)) => {
                Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                    "{}\nactive graph: `{name}`",
                    Self::activated_text(provenance)
                ))]))
            }
            Err(report) => Err(errors::tool_error(&format!("{report:?}"))),
        }
    }

    #[tool(
        description = "Bind an existing graph as your active graph — subsequent transact/query calls hit it. The graph must already exist (instantiate_graph or create_graph makes one). Attaching records who attached and when in the graph's provenance. Another process holding the graph's file makes the attach fail with a lock error rather than corrupting anything."
    )]
    async fn attach_graph(
        &self,
        Parameters(args): Parameters<AttachGraphArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let AttachGraphArgs { name, created_by } = args;
        let created_by = created_by.unwrap_or_else(|| "unknown".to_owned());
        let store = self.store.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            store
                .attach_graph(&name, &created_by)
                .map(|p| (name.clone(), p))
        })
        .await
        .map_err(|e| errors::tool_error(&format!("attach task failed: {e}")))?;
        match outcome {
            Ok((name, provenance)) => Ok(CallToolResult::success(vec![ContentBlock::text(
                match provenance.template.as_deref() {
                    Some(template) => format!(
                        "attached graph `{name}` (template: {template})\nactive graph: `{name}`"
                    ),
                    None => format!("attached graph `{name}`\nactive graph: `{name}`"),
                },
            )])),
            Err(report) => Err(errors::tool_error(&format!("{report:?}"))),
        }
    }

    #[tool(
        description = "Run one Datalog write command against your active graph: `(transact [[:entity :attr value] ...])` to assert facts, `(retract [[:entity :attr value]])` to remove, or `(rule [(name ?a ?b) ...])` to register a rule. Rules are memory-only: they are re-applied from the graph's template on reopen, but a rule you register by hand is gone after a restart — prefer the graph's template or plain facts. SAFETY: one value per (entity, attribute) per transact — two values for the same attribute in one call can read back as one (minigraf #371); split multi-valued attributes across separate transact calls. Keep facts small (well under 4 KB): store file paths in facts, not payloads. Instantiate or attach to a graph first. Returns the transaction id."
    )]
    async fn transact(
        &self,
        Parameters(args): Parameters<TransactArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let TransactArgs { datalog, graph } = args;
        let Some(graph) = graph else {
            return Err(errors::tool_error(
                "no target graph: instantiate_graph or attach_graph first (or pass `graph` explicitly when driving the server by hand)",
            ));
        };
        let store = self.store.clone();
        let graph_for_warning = graph.clone();
        let blocking_store = self.store.clone();
        let outcome =
            tokio::task::spawn_blocking(move || blocking_store.transact(&graph, &datalog))
                .await
                .map_err(|e| errors::tool_error(&format!("transact task failed: {e}")))?;
        match outcome {
            Ok(tx_id) => {
                let warning = store
                    .warning_for(&graph_for_warning)
                    .map(|w| format!("\nwarning: {w}"));
                Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                    "committed transaction {tx_id} to `{graph_for_warning}`{}",
                    warning.unwrap_or_default()
                ))]))
            }
            Err(report) => Err(errors::tool_error(&format!("{report:?}"))),
        }
    }

    #[tool(
        description = "Run one Datalog query against your active graph: `(query [:find ?a ?b :where [?e :attr ?a] ...])`. Supports bi-temporal time travel with `:as-of <tx-count>` (transaction time) and `:valid-at <iso-8601 | :any-valid-time>` (valid time), so history and audit questions are ordinary queries. Rows come back as JSON arrays aligned with the :find variables. Instantiate or attach to a graph first."
    )]
    async fn query(
        &self,
        Parameters(args): Parameters<QueryArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let QueryArgs { datalog, graph } = args;
        let Some(graph) = graph else {
            return Err(errors::tool_error(
                "no target graph: instantiate_graph or attach_graph first (or pass `graph` explicitly when driving the server by hand)",
            ));
        };
        let store = self.store.clone();
        let graph_for_warning = graph.clone();
        let blocking_store = self.store.clone();
        let outcome = tokio::task::spawn_blocking(move || blocking_store.query(&graph, &datalog))
            .await
            .map_err(|e| errors::tool_error(&format!("query task failed: {e}")))?;
        match outcome {
            Ok(json) => {
                let warning = store
                    .warning_for(&graph_for_warning)
                    .map(|w| format!("\nwarning: {w}"));
                Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                    "{json}{}",
                    warning.unwrap_or_default()
                ))]))
            }
            Err(report) => Err(errors::tool_error(&format!("{report:?}"))),
        }
    }

    #[tool(
        description = "List every graph template available for instantiate_graph (names only, sorted). A template is a directory under the config dir's graphs folder holding a hand-editable template.dl script; sibling files beside the script (like per-step instructions) belong to the template's workflow and are not managed by this server."
    )]
    async fn list_templates(&self) -> Result<CallToolResult, ErrorData> {
        let store = self.store.clone();
        let outcome =
            tokio::task::spawn_blocking(move || templates::list_templates(store.templates_dir()))
                .await
                .map_err(|e| errors::tool_error(&format!("list task failed: {e}")))?;
        match outcome {
            Ok(names) => {
                let body = if names.is_empty() {
                    "no templates yet — create a directory with a template.dl under the \
                     config dir's graphs folder"
                        .to_owned()
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
impl ServerHandler for MinigrafServer {
    fn get_info(&self) -> InitializeResult {
        // `InitializeResult`/`ServerCapabilities` are `#[non_exhaustive]`:
        // build through `new` + the `with_*` setters, and enable tools
        // through the capabilities builder.
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities)
            .with_server_info(Implementation::new(
                "jinn-minigraf",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Bi-temporal graph state for jinn sessions. instantiate_graph or \
                 create_graph binds your active graph; transact writes facts; query \
                 reads them, including :as-of/:valid-at history. One value per \
                 (entity, attribute) per transact.",
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
/// Returns an error if the minigraf directory cannot be created or the
/// listener cannot bind.
pub async fn run(
    host: &str,
    port: u16,
    minigraf_dir: PathBuf,
    templates_dir: PathBuf,
) -> Result<(), Report<errors::MinigrafServerError>> {
    let server = MinigrafServer::new(minigraf_dir, templates_dir)?;
    let service: rmcp::transport::streamable_http_server::StreamableHttpService<
        MinigrafServer,
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
        .change_context(errors::MinigrafServerError::StoreDir(format!(
            "could not bind {host}:{port}"
        )))?;
    let bound = listener.local_addr().ok();
    tracing::info!(addr = ?bound, "jinn-minigraf: serving MCP at /mcp");

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .change_context(errors::MinigrafServerError::StoreDir(
            "serving /mcp failed".to_owned(),
        ))?;
    Ok(())
}
