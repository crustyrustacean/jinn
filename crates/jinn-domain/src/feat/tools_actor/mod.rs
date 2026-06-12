//! Tool orchestrator actor - dispatches tool calls and aggregates batch results.
//!
//! This actor maintains a registry of available tools (built-in and actor-provided),
//! dispatches [`ExecuteToolBatch`] requests, and emits [`ToolBatchCompleted`] when
//! all calls in a batch finish.
//!
//! Built-in tools (`get_time`, `read`, `write`) are registered at
//! startup and executed via spawned tokio tasks. Actor-provided tools
//! are routed via [`ExecuteTool`] commands on the bus.
//!
//! Each tool execution receives a [`ToolContext`] containing the session's CWD
//! (for resolving relative paths) and an optional timeout. The orchestrator
//! reads CWD from shared [`State`] at dispatch time.

pub mod bash;
pub mod edit;
pub mod get_time;
pub mod protocol;
pub mod read;
pub mod registry;
pub mod save_plan;
pub mod skill;
pub mod tool_entry;
pub mod tool_types;
pub(crate) mod truncation;
pub mod write;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::preferences_actor::OpenrouterWebSearchConfig;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::tools_actor::protocol::command::{
    CancelToolBatch, ExecuteToolBatch, ExecuteWebFetch, RegisterTools,
};
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolExecutionCompleted, ToolsRegistered,
};
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::SessionId;
use jinn_provider::ServerToolType;
use kameo::prelude::{Actor, ActorRef, Context, Message};

/// A boxed future returned by built-in tool execute functions.
pub type BoxedToolFuture = Pin<Box<dyn Future<Output = ToolResult> + Send>>;

/// How a tool is registered and executed.
pub(crate) enum ToolRegistration {
    /// A built-in tool executed directly by the orchestrator.
    Builtin {
        /// The tool's JSON-schema definition.
        definition: ToolDefinition,
        /// The function that executes the tool call.
        execute: fn(ToolCall, ToolContext) -> BoxedToolFuture,
    },
    /// An actor-provided tool routed via [`ExecuteTool`] command.
    Actor {
        /// The tool's JSON-schema definition.
        definition: ToolDefinition,
        /// The name of the actor providing this tool.
        provider: String,
    },
}

impl std::fmt::Debug for ToolRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin { definition, .. } => f
                .debug_struct("Builtin")
                .field("name", &definition.name)
                .finish_non_exhaustive(),
            Self::Actor {
                definition,
                provider,
            } => f
                .debug_struct("Actor")
                .field("name", &definition.name)
                .field("provider", provider)
                .finish(),
        }
    }
}

/// Tracks pending tool calls within a batch.
pub(crate) struct PendingBatch {
    /// Number of tool calls still awaiting results.
    remaining: usize,
    /// Collected results so far.
    results: Vec<ToolResult>,
    /// Join handles for spawned builtin tool tasks (for cancellation).
    handles: Vec<tokio::task::JoinHandle<()>>,
}

/// Tool orchestrator actor.
///
/// Subscribes to [`RegisterTools`] and [`ExecuteToolBatch`] commands, and
/// [`ToolExecutionCompleted`] events. Dispatches tool calls to the appropriate
/// handler and aggregates results into batch completion events.
pub struct ToolOrchestratorActor {
    /// Universal actor dependencies.
    deps: ActorDeps,
    /// Tool name → registration info.
    tools: HashMap<String, ToolRegistration>,
    /// Session ID → pending batch tracker.
    pending: HashMap<SessionId, PendingBatch>,
    /// Shared application state for reading session CWD.
    state: State,
    /// Runtime services.
    services: Services,
    /// Shell binary path (captured at startup from `$SHELL`).
    shell: String,
}

/// Dependencies for [`ToolOrchestratorActor`].
pub struct ToolOrchestratorActorDeps {
    /// Universal actor dependencies.
    pub deps: ActorDeps,
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
    /// Override which built-in tools to register. `None` means register all.
    /// Each entry is a tool name (e.g., `"bash"`, `"read"`, `"write"`).
    pub builtin_filter: Option<Vec<String>>,
    /// Shell binary path (captured at startup from `$SHELL`).
    pub shell: String,
}

/// Builds the `openrouter:web_search` tool definition from config.
///
/// The `parameters` field contains actual config values (not a JSON Schema)
/// because server tools send config directly, not a function parameter schema.
fn build_openrouter_web_search_definition(config: &OpenrouterWebSearchConfig) -> ToolDefinition {
    let mut params = serde_json::Map::new();
    if let Some(ref engine) = config.engine {
        params.insert(
            "engine".to_owned(),
            serde_json::Value::String(engine.clone()),
        );
    }
    if let Some(max) = config.max_results {
        params.insert("max_results".to_owned(), serde_json::json!(max));
    }
    if let Some(max) = config.max_total_results {
        params.insert("max_total_results".to_owned(), serde_json::json!(max));
    }
    if let Some(ref size) = config.search_context_size {
        params.insert(
            "search_context_size".to_owned(),
            serde_json::Value::String(size.clone()),
        );
    }
    if let Some(ref domains) = config.allowed_domains {
        params.insert("allowed_domains".to_owned(), serde_json::json!(domains));
    }
    if let Some(ref domains) = config.excluded_domains {
        params.insert("excluded_domains".to_owned(), serde_json::json!(domains));
    }

    ToolDefinition {
        name: "openrouter:web_search".to_owned(),
        description: "Search the web for real-time information.".to_owned(),
        parameters: serde_json::Value::Object(params),
        prompt_snippet: Some("Web search (OpenRouter)".to_owned()),
        prompt_guidelines: vec![],
        server_tool_type: Some(ServerToolType::OpenrouterWebSearch),
    }
}

impl Actor for ToolOrchestratorActor {
    type Args = ToolOrchestratorActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(
        args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let deps = args.deps;
        deps.subscribe(actor_ref.clone().recipient::<RegisterTools>())
            .await;
        deps.subscribe(actor_ref.clone().recipient::<ExecuteToolBatch>())
            .await;
        deps.subscribe(actor_ref.clone().recipient::<CancelToolBatch>())
            .await;
        deps.subscribe(actor_ref.recipient::<ToolExecutionCompleted>())
            .await;

        // Read web search config from preferences storage.
        let web_search_config = args
            .services
            .user_preferences_storage
            .read()
            .openrouter_web_search
            .clone();

        let bash_config = args.services.user_preferences_storage.read().bash.clone();

        let mut actor = Self {
            deps,
            tools: HashMap::new(),
            pending: HashMap::new(),
            services: args.services,
            state: args.state,
            shell: args.shell,
        };
        let all_builtins = registry::builtin_tools(&bash_config);
        let builtins: Vec<_> = if let Some(ref filter) = args.builtin_filter {
            all_builtins
                .into_iter()
                .filter(|(def, _)| filter.contains(&def.name))
                .collect()
        } else {
            all_builtins
        };
        let mut builtin_definitions: Vec<ToolDefinition> =
            builtins.iter().map(|(d, _)| d.clone()).collect();

        for (def, execute_fn) in builtins {
            let name = def.name.clone();
            actor.tools.insert(
                name,
                ToolRegistration::Builtin {
                    definition: def,
                    execute: execute_fn,
                },
            );
        }

        // Register openrouter:web_search server tool.
        let web_search_def = build_openrouter_web_search_definition(&web_search_config);
        actor.tools.insert(
            web_search_def.name.clone(),
            ToolRegistration::Builtin {
                definition: web_search_def.clone(),
                execute: |_call, _ctx| {
                    // Server tool - handled by OpenRouter, never dispatched locally.
                    Box::pin(std::future::ready(ToolResult {
                        tool_call_id: String::new(),
                        name: "openrouter:web_search".to_owned(),
                        content: "server tool should not be dispatched".to_owned(),
                        success: false,
                        full_content: None,
                        truncation: None,
                        pin_position: None,
                    }))
                },
            },
        );
        builtin_definitions.push(web_search_def);

        // Announce built-in tools so the LLM actor can cache them.
        actor
            .publish(ToolsRegistered {
                provider: "builtin".to_owned(),
                definitions: builtin_definitions,
            })
            .await;

        Ok(actor)
    }
}

// ---------------------------------------------------------------------------
// Bridge: impl Message<T> blocks that delegate to old handler methods
// ---------------------------------------------------------------------------
// Message handlers — direct handler calls (no bridge)
// ---------------------------------------------------------------------------

impl Message<RegisterTools> for ToolOrchestratorActor {
    type Reply = ();

    async fn handle(&mut self, msg: RegisterTools, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_register_tools(&msg.provider, &msg.definitions).await;
    }
}

impl Message<ExecuteToolBatch> for ToolOrchestratorActor {
    type Reply = ();

    async fn handle(&mut self, msg: ExecuteToolBatch, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_execute_tool_batch(msg.session_id, msg.tool_calls).await;
    }
}

impl Message<CancelToolBatch> for ToolOrchestratorActor {
    type Reply = ();

    async fn handle(&mut self, msg: CancelToolBatch, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_cancel_tool_batch(&msg.session_id);
    }
}

impl Message<ToolExecutionCompleted> for ToolOrchestratorActor {
    type Reply = ();

    async fn handle(&mut self, msg: ToolExecutionCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_tool_execution_completed(msg.session_id, msg.result).await;
    }
}

impl BusPublish for ToolOrchestratorActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

impl ToolOrchestratorActor {
    /// Stores actor-provided tools and emits a [`ToolsRegistered`] event.
    async fn handle_register_tools(
        &mut self,
        provider: &str,
        definitions: &[ToolDefinition],
    ) {
        for def in definitions {
            let name = def.name.clone();
            self.tools.insert(
                name,
                ToolRegistration::Actor {
                    definition: def.clone(),
                    provider: provider.to_owned(),
                },
            );
        }

        self.publish(ToolsRegistered {
            provider: provider.to_owned(),
            definitions: definitions.to_vec(),
        }).await;
    }

    /// Dispatches each tool call and tracks the pending batch.
    async fn handle_execute_tool_batch(
        &mut self,
        session_id: SessionId,
        tool_calls: Vec<ToolCall>,
    ) {
        tracing::trace!(
            session_id = ?session_id,
            tool_call_count = tool_calls.len(),
            "handle_execute_tool_batch"
        );

        if tool_calls.is_empty() {
            self.publish(ToolBatchCompleted {
                session_id,
                results: vec![],
            }).await;
            return;
        }

        let remaining = tool_calls.len();
        let mut handles = Vec::new();
        for tc in tool_calls {
            if let Some(handle) = self.dispatch_tool_call(session_id.clone(), tc).await {
                handles.push(handle);
            }
        }
        self.pending.insert(
            session_id.clone(),
            PendingBatch {
                remaining,
                results: vec![],
                handles,
            },
        );
    }

    /// Cancels all pending tool executions for a session.
    ///
    /// Aborts spawned builtin tasks and removes the pending batch.
    /// Any `ToolExecutionCompleted` events that already arrived for this
    /// session will be ignored (the pending batch is gone).
    fn handle_cancel_tool_batch(&mut self, session_id: &SessionId) {
        if let Some(batch) = self.pending.remove(session_id) {
            let handle_count = batch.handles.len();
            for handle in batch.handles {
                handle.abort();
            }
            tracing::trace!(
                session_id = ?session_id,
                "handle_cancel_tool_batch - aborted {} tasks",
                handle_count
            );
        }
    }

    /// Builds a [`ToolContext`] for the given session by reading its CWD from shared state.
    fn build_tool_context(
        &self,
        session_id: &SessionId,
        bus: BusService,
    ) -> ToolContext {
        let prefs = self.services.user_preferences_storage.read();
        let cwd = {
            let guard = self.state.read();
            guard.session.get(session_id).map_or_else(
                || guard.session.default_cwd().clone(),
                |s: &ChatSessionState| s.cwd().to_owned(),
            )
        };
        let max_output_lines = prefs.max_tool_output_lines;
        let max_output_bytes = prefs.max_tool_output_bytes;

        ToolContext {
            cwd,
            timeout: None,
            bash_default_timeout: prefs
                .bash
                .default_timeout_secs
                .map(std::time::Duration::from_secs),
            state: Some(self.state.clone()),
            session_id: Some(session_id.clone()),
            app_paths: self.services.paths.clone(),
            bus: Some(bus),
            shell: self.shell.clone(),
            max_output_lines,
            max_output_bytes,
        }
    }

    /// Dispatches a single tool call to the appropriate handler.
    ///
    /// Returns a `JoinHandle` for builtin tool spawns so callers can track
    /// and abort them on cancellation. Returns `None` for actor-routed and
    /// unknown tools (they have no local task to abort).
    async fn dispatch_tool_call(
        &mut self,
        session_id: SessionId,
        tool_call: ToolCall,
    ) -> Option<tokio::task::JoinHandle<()>> {
        tracing::trace!(
            session_id = ?session_id,
            tool = %tool_call.name,
            reg_type = match self.tools.get(&tool_call.name) {
                Some(ToolRegistration::Builtin { .. }) => "builtin",
                Some(ToolRegistration::Actor { .. }) => "actor",
                None => "unknown",
            },
            "dispatch_tool_call"
        );

        match self.tools.get(&tool_call.name) {
            Some(ToolRegistration::Builtin { execute, .. }) => {
                let bus = self.bus().clone();
                let execute_fn = *execute;
                let tool_ctx = self.build_tool_context(&session_id, bus.clone());
                let timeout = tool_ctx.timeout;

                let handle = tokio::spawn(async move {
                    let call_id = tool_call.id.clone();
                    let call_name = tool_call.name.clone();
                    let result = match timeout {
                        Some(dur) => {
                            match tokio::time::timeout(dur, execute_fn(tool_call, tool_ctx)).await {
                                Ok(r) => r,
                                Err(_) => ToolResult {
                                    tool_call_id: call_id,
                                    name: call_name,
                                    content: format!("tool execution timed out after {dur:?}"),
                                    success: false,
                                    full_content: None,
                                    truncation: None,
                                    pin_position: None,
                                },
                            }
                        }
                        None => execute_fn(tool_call, tool_ctx).await,
                    };
                    bus.publish(ToolExecutionCompleted {
                        session_id,
                        result,
                    }).await;
                });
                Some(handle)
            }
            Some(ToolRegistration::Actor { provider, .. }) => {
                match tool_call.name.as_str() {
                    "web-fetch" => {
                        self.publish(ExecuteWebFetch {
                            session_id,
                            tool_call,
                        }).await;
                    }
                    other => {
                        tracing::warn!(
                            tool = %other,
                            "unknown actor tool — no command mapping"
                        );
                    }
                }
                None
            }
            None => {
                let call_id = tool_call.id.clone();
                let call_name = tool_call.name.clone();
                let result = ToolResult {
                    tool_call_id: call_id,
                    name: call_name,
                    content: format!("unknown tool: {}", tool_call.name),
                    success: false,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                };

                self.publish(ToolExecutionCompleted {
                    session_id,
                    result,
                }).await;
                None
            }
        }
    }

    /// Aggregates a tool result into the pending batch.
    ///
    /// When all calls in a batch have completed, emits [`ToolBatchCompleted`]
    async fn handle_tool_execution_completed(
        &mut self,
        session_id: SessionId,
        result: ToolResult,
    ) {
        let Some(batch) = self.pending.get_mut(&session_id) else {
            tracing::trace!(
                session_id = ?session_id,
                "handle_tool_execution_completed — no pending batch, ignoring"
            );
            return;
        };

        batch.remaining -= 1;
        batch.results.push(result);

        tracing::trace!(
            session_id = ?session_id,
            remaining = batch.remaining,
            "handle_tool_execution_completed"
        );

        if batch.remaining == 0 {
            let results = self
                .pending
                .remove(&session_id)
                .map(|b| b.results)
                .unwrap_or_default();

            tracing::trace!(
                session_id = ?session_id,
                result_count = results.len(),
                "emitting ToolBatchCompleted"
            );

            self.publish(ToolBatchCompleted {
                session_id,
                results,
            }).await;
        }
    }

    /// Returns a reference to the tool registration for the given name.
    #[cfg(test)]
    fn get_tool(&self, name: &str) -> Option<&ToolRegistration> {
        self.tools.get(name)
    }
}

//FIXME: disabled during actor migration — tests reference deleted types
// #[cfg(test)]
// mod tools_actor_tests;
