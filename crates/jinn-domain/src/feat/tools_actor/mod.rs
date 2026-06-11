//! Tool orchestrator actor - dispatches tool calls and aggregates batch results.
//!
//! This actor maintains a registry of available tools (built-in and actor-provided),
//! dispatches [`ExecuteToolBatch`] requests, and emits [`ToolBatchCompleted`] when
//! all calls in a batch finish.
//!
//! Built-in tools (`get_time`, `read`, `write`) are registered at
//! activation and executed via spawned tokio tasks. Actor-provided tools
//! are routed via [`ExecuteTool`] commands on the bus.
//!
//! Each tool execution receives a [`ToolContext`] containing the session's CWD
//! (for resolving relative paths) and an optional timeout. The orchestrator
//! reads CWD from shared [`State`] at dispatch time.

pub mod bash;
pub mod edit;
pub mod get_time;
pub mod grep;
pub mod protocol;
pub mod read;
pub mod registry;
pub mod save_plan;
pub mod session_query;
pub mod skill;
pub mod tool_entry;
pub mod tool_types;
pub(crate) mod truncation;
pub mod write;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, MessageSink, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::plugin_system::SessionRegistryId;
use crate::feat::preferences_actor::OpenrouterWebSearchConfig;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::tools_actor::protocol::command::{
    CancelToolBatch, ExecuteToolBatch, ExecuteWebFetch, RegisterPluginTools, RegisterTools,
};
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolExecutionCompleted, ToolsRegistered,
};
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::{Command, Event, SessionId};
use jiff::Timestamp;
use jinn_provider::ServerToolType;

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
    /// A plugin-defined tool routed to the plugin system's async thread.
    Plugin {
        /// The tool's JSON-schema definition.
        definition: ToolDefinition,
        /// `None` for global plugins, `Some(id)` for session-attached plugins.
        target: Option<SessionRegistryId>,
        /// The name of the plugin that owns this tool.
        plugin_name: String,
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
            Self::Plugin {
                definition,
                target,
                plugin_name,
            } => f
                .debug_struct("Plugin")
                .field("name", &definition.name)
                .field("target", target)
                .field("plugin_name", plugin_name)
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
    /// Shared application state.
    pub state: State,
    /// Application paths for working directory.
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
    type Message = NoDirectMsg;
    type Deps = ToolOrchestratorActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Dispatches and manages tool execution");
        ctx.subscribe_command::<RegisterTools>();
        ctx.subscribe_command::<RegisterPluginTools>();
        ctx.subscribe_command::<ExecuteToolBatch>();
        ctx.subscribe_command::<CancelToolBatch>();
        ctx.subscribe_event::<ToolExecutionCompleted>();

        // Read web search config from preferences storage.

        let web_search_config = deps
            .services
            .user_preferences_storage
            .read()
            .openrouter_web_search
            .clone();

        let bash_config = deps.services.user_preferences_storage.read().bash.clone();

        let mut actor = Self {
            tools: HashMap::new(),
            pending: HashMap::new(),
            services: deps.services,
            state: deps.state,
            shell: deps.shell,
        };
        let all_builtins = registry::builtin_tools(&bash_config);
        let builtins: Vec<_> = if let Some(ref filter) = deps.builtin_filter {
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
        if let Err(e) = ctx.send_event(Event::ToolsRegistered(ToolsRegistered {
            provider: "builtin".to_owned(),
            definitions: builtin_definitions,
        })) {
            tracing::warn!(err = ?e, "failed to emit ToolsRegistered for built-in tools");
        }

        actor
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx),
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx),
            _ => {}
        }
    }
}

impl ToolOrchestratorActor {
    /// Dispatches incoming commands to the appropriate handler.
    fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        match command {
            Command::RegisterTools(payload) => {
                self.handle_register_tools(&payload.provider, &payload.definitions, ctx);
            }
            Command::RegisterPluginTools(payload) => {
                self.handle_register_plugin_tools(
                    &payload.plugin_name,
                    &payload.target,
                    &payload.definitions,
                    ctx,
                );
            }
            Command::ExecuteToolBatch(payload) => {
                self.handle_execute_tool_batch(
                    payload.session_id.clone(),
                    payload.tool_calls.clone(),
                    payload.dispatched_at,
                    ctx,
                );
            }
            Command::CancelToolBatch(payload) => {
                self.handle_cancel_tool_batch(&payload.session_id);
            }
            _ => {}
        }
    }

    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::ToolExecutionCompleted(payload) => {
                self.handle_tool_execution_completed(
                    payload.session_id.clone(),
                    payload.result.clone(),
                    ctx,
                );
            }
            _ => {}
        }
    }

    /// Stores actor-provided tools and emits a [`ToolsRegistered`] event.
    fn handle_register_tools(
        &mut self,
        provider: &str,
        definitions: &[ToolDefinition],
        ctx: &ActorContext,
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

        if let Err(e) = ctx.send_event(Event::ToolsRegistered(ToolsRegistered {
            provider: provider.to_owned(),
            definitions: definitions.to_vec(),
        })) {
            tracing::warn!(err = ?e, "failed to emit ToolsRegistered event");
        }
    }

    /// Registers tool definitions from a Lua plugin.
    fn handle_register_plugin_tools(
        &mut self,
        plugin_name: &str,
        target: &Option<SessionRegistryId>,
        definitions: &[ToolDefinition],
        ctx: &ActorContext,
    ) {
        for def in definitions {
            let name = def.name.clone();
            self.tools.insert(
                name,
                ToolRegistration::Plugin {
                    definition: def.clone(),
                    target: *target,
                    plugin_name: plugin_name.to_owned(),
                },
            );
        }

        if let Err(e) = ctx.send_event(Event::ToolsRegistered(ToolsRegistered {
            provider: format!("plugin:{plugin_name}"),
            definitions: definitions.to_vec(),
        })) {
            tracing::warn!(err = ?e, "failed to emit ToolsRegistered event for plugin");
        }
    }

    /// Dispatches each tool call and tracks the pending batch.
    fn handle_execute_tool_batch(
        &mut self,
        session_id: SessionId,
        tool_calls: Vec<ToolCall>,
        dispatched_at: Timestamp,
        ctx: &ActorContext,
    ) {
        tracing::trace!(
            session_id = ?session_id,
            tool_call_count = tool_calls.len(),
            "handle_execute_tool_batch"
        );

        if tool_calls.is_empty() {
            if let Err(e) = ctx.send_event(Event::ToolBatchCompleted(ToolBatchCompleted {
                session_id,
                results: vec![],
            })) {
                tracing::warn!(err = ?e, "failed to emit empty ToolBatchCompleted");
            }
            return;
        }

        let remaining = tool_calls.len();
        let mut handles = Vec::new();
        for tc in tool_calls {
            if let Some(handle) =
                self.dispatch_tool_call(session_id.clone(), tc, dispatched_at, ctx)
            {
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
        sink: std::sync::Arc<dyn MessageSink>,
        dispatched_at: Timestamp,
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
            sink: Some(sink),
            shell: self.shell.clone(),
            max_output_lines,
            max_output_bytes,
            dispatched_at,
        }
    }

    /// Dispatches a single tool call to the appropriate handler.
    ///
    /// Returns a `JoinHandle` for builtin tool spawns so callers can track
    /// and abort them on cancellation. Returns `None` for actor-routed and
    /// unknown tools (they have no local task to abort).
    fn dispatch_tool_call(
        &self,
        session_id: SessionId,
        tool_call: ToolCall,
        dispatched_at: Timestamp,
        ctx: &ActorContext,
    ) -> Option<tokio::task::JoinHandle<()>> {
        tracing::trace!(
            session_id = ?session_id,
            tool = %tool_call.name,
            reg_type = match self.tools.get(&tool_call.name) {
                Some(ToolRegistration::Builtin { .. }) => "builtin",
                Some(ToolRegistration::Actor { .. }) => "actor",
                Some(ToolRegistration::Plugin { .. }) => "plugin",
                None => "unknown",
            },
            "dispatch_tool_call"
        );

        match self.tools.get(&tool_call.name) {
            Some(ToolRegistration::Builtin { execute, .. }) => {
                let sink = ctx.sink();
                let execute_fn = *execute;
                let tool_ctx = self.build_tool_context(&session_id, sink.clone(), dispatched_at);
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
                    if let Err(e) =
                        sink.send_event(Event::ToolExecutionCompleted(ToolExecutionCompleted {
                            session_id,
                            result,
                        }))
                    {
                        tracing::warn!(
                            err = ?e,
                            "builtin tool failed to send ToolExecutionCompleted"
                        );
                    }
                });
                Some(handle)
            }
            Some(ToolRegistration::Actor { provider, .. }) => {
                let cmd = match tool_call.name.as_str() {
                    "web-fetch" => Command::ExecuteWebFetch(ExecuteWebFetch {
                        session_id,
                        tool_call,
                    }),
                    other => {
                        tracing::warn!(
                            tool = %other,
                            "unknown actor tool \u{2014} no command mapping"
                        );
                        return None;
                    }
                };
                if let Err(e) = ctx.send_command(cmd) {
                    tracing::warn!(
                        err = ?e,
                        provider = %provider,
                        "failed to send actor tool command"
                    );
                }
                None
            }
            Some(ToolRegistration::Plugin {
                target,
                plugin_name,
                ..
            }) => {
                let sink = ctx.sink();
                let _plugin_fire = self.services.plugins.clone();
                let target = target.clone();
                let sid = session_id.clone();
                let plugin_name = plugin_name.clone();
                let plugin_fire = self.services.plugins.clone();
                let target = target.clone();
                let plugin_name = plugin_name.clone();
                let arguments: serde_json::Value =
                    serde_json::from_str(&tool_call.arguments).unwrap_or_default();

                let handle = tokio::spawn(async move {
                    let result = match plugin_fire
                        .execute_plugin_tool(
                            target,
                            &sid,
                            &plugin_name,
                            &tool_call.name,
                            &arguments,
                        )
                        .await
                    {
                        Ok(content) => ToolResult {
                            tool_call_id: tool_call.id.clone(),
                            name: tool_call.name.clone(),
                            content,
                            success: true,
                            full_content: None,
                            truncation: None,
                            pin_position: None,
                        },
                        Err(report) => {
                            tracing::warn!(?report, %plugin_name, "plugin tool execution failed");
                            ToolResult {
                                tool_call_id: tool_call.id.clone(),
                                name: tool_call.name.clone(),
                                content: format!("plugin tool error: {report:#}"),
                                success: false,
                                full_content: None,
                                truncation: None,
                                pin_position: None,
                            }
                        }
                    };
                    if let Err(e) =
                        sink.send_event(Event::ToolExecutionCompleted(ToolExecutionCompleted {
                            session_id,
                            result,
                        }))
                    {
                        tracing::warn!(err = ?e, "plugin tool failed to send ToolExecutionCompleted");
                    }
                });
                Some(handle)
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

                if let Err(e) =
                    ctx.send_event(Event::ToolExecutionCompleted(ToolExecutionCompleted {
                        session_id,
                        result,
                    }))
                {
                    tracing::warn!(
                        err = ?e,
                        "failed to send unknown-tool ToolExecutionCompleted"
                    );
                }
                None
            }
        }
    }

    /// Aggregates a tool result into the pending batch.
    ///
    /// When all calls in a batch have completed, emits [`ToolBatchCompleted`]
    fn handle_tool_execution_completed(
        &mut self,
        session_id: SessionId,
        result: ToolResult,
        ctx: &ActorContext,
    ) {
        let Some(batch) = self.pending.get_mut(&session_id) else {
            tracing::warn!(
                session_id = ?session_id,
                "received ToolExecutionCompleted for unknown session"
            );
            return;
        };

        batch.results.push(result);
        batch.remaining -= 1;

        tracing::trace!(
            session_id = ?session_id,
            remaining = batch.remaining,
            "handle_tool_execution_completed"
        );

        if batch.remaining == 0 {
            // unwrap: we just checked the entry exists above.
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

            if let Err(e) = ctx.send_event(Event::ToolBatchCompleted(ToolBatchCompleted {
                session_id,
                results,
            })) {
                tracing::warn!(err = ?e, "failed to emit ToolBatchCompleted");
            }
        }
    }

    /// Returns a reference to the tool registration for the given name.
    #[cfg(test)]
    fn get_tool(&self, name: &str) -> Option<&ToolRegistration> {
        self.tools.get(name)
    }
}

#[cfg(test)]
mod tools_actor_tests;
