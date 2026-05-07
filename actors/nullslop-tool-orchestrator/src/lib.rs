//! Tool orchestrator actor — dispatches tool calls and aggregates batch results.
//!
//! This actor maintains a registry of available tools (built-in, actor-provided,
//! and workflow creation), dispatches [`ExecuteToolBatch`] requests, and emits
//! [`ToolBatchCompleted`] when all calls in a batch finish.
//!
//! Built-in tools (`echo`, `get_time`, `file_read`, `file_write`) are registered at
//! activation and executed via spawned tokio tasks. Actor-provided tools
//! are routed via [`ExecuteTool`] commands on the bus. Workflow creation tools
//! (`workflow_create`, `workflow_add_step`, etc.) are executed synchronously
//! against an in-progress [`WorkflowBuilder`] draft.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use nullslop_protocol::tool::{
    ExecuteTool, ExecuteToolBatch, RegisterTools, ToolBatchCompleted, ToolCall, ToolDefinition,
    ToolExecutionCompleted, ToolResult, ToolsRegistered,
};
use nullslop_protocol::workflow::LoadWorkflow;
use nullslop_protocol::{Command, Event, SessionId};
use nullslop_workflow::WorkflowBuilder;
use nullslop_workflow::definition::{ModelHint, StepDef, StepOutputDef};
use nullslop_workflow::guard::{GuardExpr, GuardPredicate};
use nullslop_workflow_store::WorkflowStoreService;

/// A boxed future returned by built-in tool execute functions.
type BoxedToolFuture = Pin<Box<dyn Future<Output = ToolResult> + Send>>;

/// How a tool is registered and executed.
enum ToolRegistration {
    /// A built-in tool executed directly by the orchestrator.
    Builtin {
        /// The tool's JSON-schema definition.
        definition: ToolDefinition,
        /// The function that executes the tool call.
        execute: fn(ToolCall) -> BoxedToolFuture,
    },
    /// An actor-provided tool routed via [`ExecuteTool`] command.
    Actor {
        /// The tool's JSON-schema definition.
        definition: ToolDefinition,
        /// The name of the actor providing this tool.
        provider: String,
    },
    /// A workflow creation tool executed synchronously by the orchestrator.
    Workflow {
        /// The tool's JSON-schema definition.
        definition: ToolDefinition,
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
            Self::Workflow { definition } => f
                .debug_struct("Workflow")
                .field("name", &definition.name)
                .finish(),
        }
    }
}

/// Tracks pending tool calls within a batch.
struct PendingBatch {
    /// Number of tool calls still awaiting results.
    remaining: usize,
    /// Collected results so far.
    results: Vec<ToolResult>,
}

/// Direct message type for the tool orchestrator actor.
///
/// Currently unused — the orchestrator only responds to bus commands and events.
pub enum ToolOrchestratorDirectMsg {}

/// Tool orchestrator actor.
///
/// Subscribes to [`RegisterTools`] and [`ExecuteToolBatch`] commands, and
/// [`ToolExecutionCompleted`] events. Dispatches tool calls to the appropriate
/// handler and aggregates results into batch completion events.
///
/// Workflow creation tools are executed synchronously against the in-progress
/// [`WorkflowBuilder`] draft stored in [`workflow_builder`](Self::workflow_builder).
pub struct ToolOrchestratorActor {
    /// Tool name → registration info.
    tools: HashMap<String, ToolRegistration>,
    /// Session ID → pending batch tracker.
    pending: HashMap<SessionId, PendingBatch>,
    /// In-progress workflow draft, if any.
    workflow_builder: Option<WorkflowBuilder>,
    /// Workflow definition store for persisting committed workflows.
    workflow_store: Option<WorkflowStoreService>,
}

impl Actor for ToolOrchestratorActor {
    type Message = ToolOrchestratorDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<RegisterTools>();
        ctx.subscribe_command::<ExecuteToolBatch>();
        ctx.subscribe_event::<ToolExecutionCompleted>();

        let workflow_store = ctx.take_data::<WorkflowStoreService>();

        let mut actor = Self {
            tools: HashMap::new(),
            pending: HashMap::new(),
            workflow_builder: None,
            workflow_store,
        };

        let builtins = builtin_tools();
        let builtin_definitions: Vec<ToolDefinition> =
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

        // Register workflow creation tools.
        let workflow_defs = workflow_tool_definitions();
        for def in workflow_defs {
            let name = def.name.clone();
            actor
                .tools
                .insert(name, ToolRegistration::Workflow { definition: def });
        }

        // Announce built-in tools so the LLM actor can cache them.
        if let Err(e) = ctx.send_event(Event::ToolsRegistered {
            payload: ToolsRegistered {
                provider: "builtin".to_owned(),
                definitions: builtin_definitions,
            },
        }) {
            tracing::warn!(err = ?e, "failed to emit ToolsRegistered for built-in tools");
        }

        // Announce workflow creation tools.
        let wf_names: Vec<&str> = actor
            .tools
            .iter()
            .filter(|(_, r)| matches!(r, ToolRegistration::Workflow { .. }))
            .map(|(n, _)| n.as_str())
            .collect();
        let wf_definitions: Vec<ToolDefinition> = actor
            .tools
            .iter()
            .filter_map(|(_, r)| match r {
                ToolRegistration::Workflow { definition } => Some(definition.clone()),
                _ => None,
            })
            .collect();

        if !wf_definitions.is_empty()
            && let Err(e) = ctx.send_event(Event::ToolsRegistered {
                payload: ToolsRegistered {
                    provider: "workflow".to_owned(),
                    definitions: wf_definitions,
                },
            })
        {
            tracing::warn!(
                err = ?e,
                tools = ?wf_names,
                "failed to emit ToolsRegistered for workflow tools"
            );
        }

        actor
    }

    async fn handle(&mut self, msg: ActorEnvelope<ToolOrchestratorDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx).await,
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx),
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl ToolOrchestratorActor {
    /// Dispatches incoming commands to the appropriate handler.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        match command {
            Command::RegisterTools { payload } => {
                self.handle_register_tools(&payload.provider, &payload.definitions, ctx);
            }
            Command::ExecuteToolBatch { payload } => {
                self.handle_execute_tool_batch(
                    payload.session_id.clone(),
                    payload.tool_calls.clone(),
                    ctx,
                )
                .await;
            }
            _ => {}
        }
    }

    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::ToolExecutionCompleted { payload } => {
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

        if let Err(e) = ctx.send_event(Event::ToolsRegistered {
            payload: ToolsRegistered {
                provider: provider.to_owned(),
                definitions: definitions.to_vec(),
            },
        }) {
            tracing::warn!(err = ?e, "failed to emit ToolsRegistered event");
        }
    }

    /// Dispatches each tool call and tracks the pending batch.
    ///
    /// Workflow tools (names starting with `workflow_`) are executed synchronously.
    /// Regular tools are dispatched asynchronously. If all calls are workflow tools,
    /// [`ToolBatchCompleted`] is emitted immediately. If the batch is mixed,
    /// workflow results are pre-populated in the pending batch alongside the
    /// async dispatch.
    async fn handle_execute_tool_batch(
        &mut self,
        session_id: SessionId,
        tool_calls: Vec<ToolCall>,
        ctx: &ActorContext,
    ) {
        if tool_calls.is_empty() {
            if let Err(e) = ctx.send_event(Event::ToolBatchCompleted {
                payload: ToolBatchCompleted {
                    session_id,
                    results: vec![],
                },
            }) {
                tracing::warn!(err = ?e, "failed to emit empty ToolBatchCompleted");
            }
            return;
        }

        let mut sync_results: Vec<ToolResult> = Vec::new();
        let mut async_calls: Vec<ToolCall> = Vec::new();

        for tool_call in tool_calls {
            if tool_call.name.starts_with("workflow_") {
                sync_results.push(self.execute_workflow_tool(tool_call, ctx).await);
            } else {
                async_calls.push(tool_call);
            }
        }

        if async_calls.is_empty() {
            // All synchronous — emit batch completed immediately.
            if let Err(e) = ctx.send_event(Event::ToolBatchCompleted {
                payload: ToolBatchCompleted {
                    session_id,
                    results: sync_results,
                },
            }) {
                tracing::warn!(err = ?e, "failed to emit sync ToolBatchCompleted");
            }
        } else {
            // Mixed/all async — pre-populate pending batch with sync results.
            let remaining = async_calls.len();
            self.pending.insert(
                session_id.clone(),
                PendingBatch {
                    remaining,
                    results: sync_results,
                },
            );
            for tc in async_calls {
                self.dispatch_tool_call(session_id.clone(), tc, ctx);
            }
        }
    }

    /// Dispatches a single tool call to the appropriate handler.
    fn dispatch_tool_call(&self, session_id: SessionId, tool_call: ToolCall, ctx: &ActorContext) {
        match self.tools.get(&tool_call.name) {
            Some(ToolRegistration::Builtin { execute, .. }) => {
                let sink = ctx.sink();
                let execute_fn = *execute;

                tokio::spawn(async move {
                    let result = execute_fn(tool_call).await;
                    if let Err(e) = sink.send_event(Event::ToolExecutionCompleted {
                        payload: ToolExecutionCompleted { session_id, result },
                    }) {
                        tracing::warn!(
                            err = ?e,
                            "builtin tool failed to send ToolExecutionCompleted"
                        );
                    }
                });
            }
            Some(ToolRegistration::Actor { provider, .. }) => {
                if let Err(e) = ctx.send_command(Command::ExecuteTool {
                    payload: ExecuteTool {
                        session_id,
                        tool_call,
                    },
                }) {
                    tracing::warn!(
                        err = ?e,
                        provider = %provider,
                        "failed to send ExecuteTool command"
                    );
                }
            }
            Some(ToolRegistration::Workflow { definition }) => {
                // Safety net — workflow tools should be intercepted before dispatch.
                tracing::warn!(
                    tool = %definition.name,
                    "workflow tool reached async dispatch (should be handled synchronously)"
                );
                let call_id = tool_call.id.clone();
                let call_name = tool_call.name.clone();
                let result = ToolResult {
                    tool_call_id: call_id,
                    name: call_name,
                    content: "workflow tool dispatched incorrectly".to_owned(),
                    success: false,
                };
                if let Err(e) = ctx.send_event(Event::ToolExecutionCompleted {
                    payload: ToolExecutionCompleted { session_id, result },
                }) {
                    tracing::warn!(err = ?e, "failed to send workflow fallback result");
                }
            }
            None => {
                let call_id = tool_call.id.clone();
                let call_name = tool_call.name.clone();
                let result = ToolResult {
                    tool_call_id: call_id,
                    name: call_name,
                    content: format!("unknown tool: {}", tool_call.name),
                    success: false,
                };

                if let Err(e) = ctx.send_event(Event::ToolExecutionCompleted {
                    payload: ToolExecutionCompleted { session_id, result },
                }) {
                    tracing::warn!(
                        err = ?e,
                        "failed to send unknown-tool ToolExecutionCompleted"
                    );
                }
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

        if batch.remaining == 0 {
            // unwrap: we just checked the entry exists above.
            let results = self
                .pending
                .remove(&session_id)
                .map(|b| b.results)
                .unwrap_or_default();

            if let Err(e) = ctx.send_event(Event::ToolBatchCompleted {
                payload: ToolBatchCompleted {
                    session_id,
                    results,
                },
            }) {
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

// ---------------------------------------------------------------------------
// Workflow tool execution
// ---------------------------------------------------------------------------

impl ToolOrchestratorActor {
    /// Dispatches a workflow tool call to the appropriate handler.
    async fn execute_workflow_tool(&mut self, call: ToolCall, ctx: &ActorContext) -> ToolResult {
        let call_id = call.id.clone();
        let call_name = call.name.clone();

        match call.name.as_str() {
            "workflow_create" => self.handle_workflow_create(call),
            "workflow_add_step" => self.handle_workflow_add_step(call),
            "workflow_add_guard" => self.handle_workflow_add_guard(call),
            "workflow_add_output" => self.handle_workflow_add_output(call),
            "workflow_add_global" => self.handle_workflow_add_global(call),
            "workflow_set_model_overrides" => self.handle_workflow_set_model_overrides(call),
            "workflow_preview" => self.handle_workflow_preview(call),
            "workflow_commit" => self.handle_workflow_commit(call, ctx).await,
            "workflow_abort" => self.handle_workflow_abort(call),
            _ => tool_error(call_id, call_name, "unknown workflow tool".to_owned()),
        }
    }

    /// Creates a new draft workflow.
    fn handle_workflow_create(&mut self, call: ToolCall) -> ToolResult {
        if self.workflow_builder.is_some() {
            return tool_error(
                call.id,
                call.name,
                "a draft workflow already exists. Use workflow_abort first.".to_owned(),
            );
        }

        let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                return tool_error(
                    call.id,
                    call.name,
                    format!("failed to parse arguments: {e}"),
                );
            }
        };

        let name = match string_field(&args, "name") {
            Ok(n) => n,
            Err(msg) => return tool_error(call.id, call.name, msg),
        };
        let description = match string_field(&args, "description") {
            Ok(d) => d,
            Err(msg) => return tool_error(call.id, call.name, msg),
        };

        let mut builder = WorkflowBuilder::new();
        match builder.create(name, description) {
            Ok(()) => {
                self.workflow_builder = Some(builder);
                tool_ok(call, "Draft workflow created.".to_owned())
            }
            Err(e) => tool_error(
                call.id,
                call.name,
                format!("Failed to create workflow: {e}"),
            ),
        }
    }

    /// Adds a step to the current draft workflow.
    fn handle_workflow_add_step(&mut self, call: ToolCall) -> ToolResult {
        let Some(ref mut builder) = self.workflow_builder else {
            return tool_error(
                call.id,
                call.name,
                "no draft workflow. Use workflow_create first.".to_owned(),
            );
        };

        let step = match parse_step_from_args(&call.arguments) {
            Ok(s) => s,
            Err(msg) => return tool_error(call.id, call.name, msg),
        };

        let step_id = step.id.clone();
        match builder.add_step(step) {
            Ok(()) => tool_ok(call, format!("Step '{step_id}' added.")),
            Err(e) => tool_error(call.id, call.name, format!("Failed to add step: {e}")),
        }
    }

    /// Adds a guard predicate to an existing step.
    fn handle_workflow_add_guard(&mut self, call: ToolCall) -> ToolResult {
        let Some(ref mut builder) = self.workflow_builder else {
            return tool_error(
                call.id,
                call.name,
                "no draft workflow. Use workflow_create first.".to_owned(),
            );
        };

        let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                return tool_error(
                    call.id,
                    call.name,
                    format!("failed to parse arguments: {e}"),
                );
            }
        };

        let step_id = match string_field(&args, "step_id") {
            Ok(s) => s,
            Err(msg) => return tool_error(call.id, call.name, msg),
        };

        let predicate_type = match string_field(&args, "predicate") {
            Ok(s) => s,
            Err(msg) => return tool_error(call.id, call.name, msg),
        };

        let pred_args = args
            .get("args")
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        let guard = match parse_guard_predicate(&predicate_type, &pred_args) {
            Ok(g) => g,
            Err(msg) => return tool_error(call.id, call.name, msg),
        };

        match builder.add_guard(&step_id, guard) {
            Ok(()) => tool_ok(call, format!("Guard added to step '{step_id}'.")),
            Err(e) => tool_error(call.id, call.name, format!("Failed to add guard: {e}")),
        }
    }

    /// Adds an output descriptor to an existing step.
    fn handle_workflow_add_output(&mut self, call: ToolCall) -> ToolResult {
        let Some(ref mut builder) = self.workflow_builder else {
            return tool_error(
                call.id,
                call.name,
                "no draft workflow. Use workflow_create first.".to_owned(),
            );
        };

        let (step_id, output) = match parse_output_from_args(&call.arguments) {
            Ok(pair) => pair,
            Err(msg) => return tool_error(call.id, call.name, msg),
        };

        match builder.add_output(&step_id, output) {
            Ok(()) => tool_ok(call, format!("Output added to step '{step_id}'.")),
            Err(e) => tool_error(call.id, call.name, format!("Failed to add output: {e}")),
        }
    }

    /// Adds or updates a global template variable.
    fn handle_workflow_add_global(&mut self, call: ToolCall) -> ToolResult {
        let Some(ref mut builder) = self.workflow_builder else {
            return tool_error(
                call.id,
                call.name,
                "no draft workflow. Use workflow_create first.".to_owned(),
            );
        };

        let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                return tool_error(
                    call.id,
                    call.name,
                    format!("failed to parse arguments: {e}"),
                );
            }
        };

        let key = match string_field(&args, "key") {
            Ok(k) => k,
            Err(msg) => return tool_error(call.id, call.name, msg),
        };
        let value = match string_field(&args, "value") {
            Ok(v) => v,
            Err(msg) => return tool_error(call.id, call.name, msg),
        };

        builder.add_global(key.clone(), value);
        tool_ok(call, format!("Global '{key}' set."))
    }

    /// Sets model hint → model ID mappings.
    fn handle_workflow_set_model_overrides(&mut self, call: ToolCall) -> ToolResult {
        let Some(ref mut builder) = self.workflow_builder else {
            return tool_error(
                call.id,
                call.name,
                "no draft workflow. Use workflow_create first.".to_owned(),
            );
        };

        let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                return tool_error(
                    call.id,
                    call.name,
                    format!("failed to parse arguments: {e}"),
                );
            }
        };

        let Some(obj) = args.as_object() else {
            return tool_error(call.id, call.name, "arguments must be an object".to_owned());
        };

        let mut count = 0usize;
        for (hint, value) in obj {
            if let Some(model_id) = value.as_str() {
                builder.set_model_override(hint.clone(), model_id.to_owned());
                count += 1;
            }
        }

        tool_ok(call, format!("{count} model override(s) set."))
    }

    /// Returns a human-readable preview of the current draft.
    fn handle_workflow_preview(&self, call: ToolCall) -> ToolResult {
        let Some(ref builder) = self.workflow_builder else {
            return tool_error(call.id, call.name, "no draft workflow.".to_owned());
        };
        let preview = builder.preview();
        tool_ok(call, preview)
    }

    /// Validates, builds, persists, and loads the workflow into the bus.
    ///
    /// After building the [`WorkflowDef`], the definition is saved to the
    /// workflow store (best-effort — failure is logged but does not prevent
    /// the workflow from loading).
    async fn handle_workflow_commit(&mut self, call: ToolCall, ctx: &ActorContext) -> ToolResult {
        let Some(ref builder) = self.workflow_builder else {
            return tool_error(
                call.id,
                call.name,
                "no draft workflow to commit.".to_owned(),
            );
        };

        // Validate first (non-consuming) — on failure, builder remains intact for fixes.
        if let Err(e) = builder.validate() {
            return tool_error(call.id, call.name, format!("Validation failed: {e}"));
        }

        // Build consumes the builder.
        // unwrap: validate() passed, so build() will succeed.
        let builder = self.workflow_builder.take().unwrap();
        let def = builder.build().unwrap();
        let msg = format!(
            "Workflow '{}' committed with {} steps.",
            def.name,
            def.steps.len()
        );

        // Persist the workflow definition to the global store (best-effort).
        if let Some(store) = &self.workflow_store
            && let Err(e) = store.save(&def.name, &def).await
        {
            tracing::warn!(
                workflow = %def.name,
                err = ?e,
                "failed to persist workflow definition"
            );
            // Non-fatal: the workflow still loads and runs, just isn't saved to disk.
        }

        // Send LoadWorkflow to the bus. The workflow handler picks it up and creates the state machine.
        if let Err(e) = ctx.send_command(Command::LoadWorkflow {
            payload: LoadWorkflow { definition: def },
        }) {
            tracing::warn!(err = ?e, "failed to send LoadWorkflow command");
        }

        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: msg,
            success: true,
        }
    }

    /// Discards the current draft workflow.
    fn handle_workflow_abort(&mut self, call: ToolCall) -> ToolResult {
        if self.workflow_builder.take().is_some() {
            tool_ok(call, "Draft workflow discarded.".to_owned())
        } else {
            tool_error(call.id, call.name, "no draft workflow to abort.".to_owned())
        }
    }
}

// ---------------------------------------------------------------------------
// Argument parsing helpers
// ---------------------------------------------------------------------------

/// Creates a success [`ToolResult`].
fn tool_ok(call: ToolCall, content: String) -> ToolResult {
    ToolResult {
        tool_call_id: call.id,
        name: call.name,
        content,
        success: true,
    }
}

/// Creates an error [`ToolResult`].
fn tool_error(call_id: String, name: String, content: String) -> ToolResult {
    ToolResult {
        tool_call_id: call_id,
        name,
        content,
        success: false,
    }
}

/// Extracts a required string field from a JSON value.
///
/// Returns an error message with the field name if missing or not a string.
fn string_field(value: &serde_json::Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| format!("missing or invalid required field: {field}"))
}

/// Extracts an optional string field from a JSON value.
///
/// Returns `None` if the field is missing or not a string.
fn optional_string_field(value: &serde_json::Value, field: &str) -> Option<String> {
    value.get(field).and_then(|v| v.as_str()).map(String::from)
}

/// Extracts an optional bool field from a JSON value.
///
/// Returns `false` if the field is missing or not a bool.
fn optional_bool_field(value: &serde_json::Value, field: &str) -> bool {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Parses a model hint string into a [`ModelHint`].
fn parse_model_hint(s: &str) -> Result<ModelHint, String> {
    match s {
        "small" => Ok(ModelHint::Small),
        "medium" => Ok(ModelHint::Medium),
        "large" => Ok(ModelHint::Large),
        other => Err(format!(
            "unknown model_hint '{other}'. Expected 'small', 'medium', or 'large'."
        )),
    }
}

/// Parses a guard predicate from a type name and arguments object.
fn parse_guard_predicate(
    predicate_type: &str,
    args: &serde_json::Value,
) -> Result<GuardPredicate, String> {
    match predicate_type {
        "file_exists" => {
            let path = string_field(args, "path")?;
            Ok(GuardPredicate::FileExists { path })
        }
        "dir_exists" => {
            let path = string_field(args, "path")?;
            Ok(GuardPredicate::DirExists { path })
        }
        "file_hash_matches" => {
            let path = string_field(args, "path")?;
            Ok(GuardPredicate::FileHashMatches { path })
        }
        "command_succeeds" => {
            let command = string_field(args, "command")?;
            Ok(GuardPredicate::CommandSucceeds { command })
        }
        "output_matches" => {
            let command = string_field(args, "command")?;
            let pattern = string_field(args, "pattern")?;
            Ok(GuardPredicate::OutputMatches { command, pattern })
        }
        "value_set" => {
            let variable = string_field(args, "variable")?;
            Ok(GuardPredicate::ValueSet { variable })
        }
        other => Err(format!(
            "unknown predicate type '{other}'. Expected one of: file_exists, dir_exists, file_hash_matches, command_succeeds, output_matches, value_set"
        )),
    }
}

/// Parses JSON arguments into a [`StepDef`].
fn parse_step_from_args(args_str: &str) -> Result<StepDef, String> {
    let args: serde_json::Value =
        serde_json::from_str(args_str).map_err(|e| format!("failed to parse arguments: {e}"))?;

    let id = string_field(&args, "id")?;
    let title = string_field(&args, "title")?;
    let instructions = string_field(&args, "instructions")?;
    let model_hint_str = string_field(&args, "model_hint")?;
    let model_hint = parse_model_hint(&model_hint_str)?;

    let checkpoint = optional_bool_field(&args, "checkpoint");
    let requires_user_input = optional_bool_field(&args, "requires_user_input");

    let tools: Vec<String> = args
        .get("tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(StepDef {
        id,
        title,
        instructions,
        model_hint,
        checkpoint,
        requires_user_input,
        tools,
        guards: GuardExpr::None,
        outputs: vec![],
        depends_on: vec![],
    })
}

/// Parses JSON arguments into a step ID and [`StepOutputDef`].
fn parse_output_from_args(args_str: &str) -> Result<(String, StepOutputDef), String> {
    let args: serde_json::Value =
        serde_json::from_str(args_str).map_err(|e| format!("failed to parse arguments: {e}"))?;

    let step_id = string_field(&args, "step_id")?;
    let kind = string_field(&args, "kind")?;
    let label = string_field(&args, "label")?;

    let output = match kind.as_str() {
        "file" => {
            let Some(path) = optional_string_field(&args, "path") else {
                return Err("field 'path' is required for kind 'file'".to_owned());
            };
            StepOutputDef::File { label, path }
        }
        "summary" => {
            let Some(value) = optional_string_field(&args, "value") else {
                return Err("field 'value' is required for kind 'summary'".to_owned());
            };
            StepOutputDef::Summary { label, value }
        }
        "artifact" => {
            let Some(description) = optional_string_field(&args, "description") else {
                return Err("field 'description' is required for kind 'artifact'".to_owned());
            };
            StepOutputDef::Artifact { label, description }
        }
        other => {
            return Err(format!(
                "unknown output kind '{other}'. Expected 'file', 'summary', or 'artifact'."
            ));
        }
    };

    Ok((step_id, output))
}

// ---------------------------------------------------------------------------
// Built-in tools
// ---------------------------------------------------------------------------

/// A built-in tool entry: its definition paired with its execute function.
type BuiltinToolEntry = (ToolDefinition, fn(ToolCall) -> BoxedToolFuture);

/// Returns the built-in tool definitions and their execute functions.
fn builtin_tools() -> Vec<BuiltinToolEntry> {
    vec![
        (
            echo_definition(),
            execute_echo as fn(ToolCall) -> BoxedToolFuture,
        ),
        (
            get_time_definition(),
            execute_get_time as fn(ToolCall) -> BoxedToolFuture,
        ),
        (
            file_read_definition(),
            execute_file_read as fn(ToolCall) -> BoxedToolFuture,
        ),
        (
            file_write_definition(),
            execute_file_write as fn(ToolCall) -> BoxedToolFuture,
        ),
    ]
}

/// Returns the tool definition for the `echo` built-in tool.
fn echo_definition() -> ToolDefinition {
    ToolDefinition {
        name: "echo".to_owned(),
        description: "Echoes the input text back as the result.".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Text to echo back"
                }
            },
            "required": ["input"]
        }),
    }
}

/// Returns the tool definition for the `get_time` built-in tool.
fn get_time_definition() -> ToolDefinition {
    ToolDefinition {
        name: "get_time".to_owned(),
        description: "Returns the current date and time in UTC.".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    }
}

/// Returns the tool definition for the `file_read` built-in tool.
fn file_read_definition() -> ToolDefinition {
    ToolDefinition {
        name: "file_read".to_owned(),
        description: "Reads the contents of a file from disk.".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                }
            },
            "required": ["path"]
        }),
    }
}

/// Executes the `echo` built-in tool.
fn execute_echo(call: ToolCall) -> BoxedToolFuture {
    Box::pin(async move {
        match serde_json::from_str::<serde_json::Value>(&call.arguments) {
            Ok(args) => {
                let input = args.get("input").and_then(|v| v.as_str()).unwrap_or("");
                ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: input.to_owned(),
                    success: true,
                }
            }
            Err(e) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to parse arguments: {e}"),
                success: false,
            },
        }
    })
}

/// Executes the `get_time` built-in tool.
fn execute_get_time(call: ToolCall) -> BoxedToolFuture {
    Box::pin(async move {
        let now = jiff::Zoned::now();
        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: now.to_string(),
            success: true,
        }
    })
}

/// Returns the tool definition for the `file_write` built-in tool.
fn file_write_definition() -> ToolDefinition {
    ToolDefinition {
        name: "file_write".to_owned(),
        description: "Writes content to a file on disk, creating parent directories as needed."
            .to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        }),
    }
}

/// Executes the `file_read` built-in tool using async I/O.
fn execute_file_read(call: ToolCall) -> BoxedToolFuture {
    Box::pin(async move {
        let path = match serde_json::from_str::<serde_json::Value>(&call.arguments) {
            Ok(args) => args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to parse arguments: {e}"),
                    success: false,
                };
            }
        };

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content,
                success: true,
            },
            Err(e) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to read file '{path}': {e}"),
                success: false,
            },
        }
    })
}

/// Executes the `file_write` built-in tool using async I/O.
///
/// Creates parent directories if they don't exist. Overwrites the file if it
/// already exists.
fn execute_file_write(call: ToolCall) -> BoxedToolFuture {
    Box::pin(async move {
        let (path, content) = match serde_json::from_str::<serde_json::Value>(&call.arguments) {
            Ok(args) => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                (path, content)
            }
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to parse arguments: {e}"),
                    success: false,
                };
            }
        };

        if let Some(parent) = std::path::Path::new(&path).parent()
            && !parent.as_os_str().is_empty()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to create parent directories for '{path}': {e}"),
                success: false,
            };
        }

        match tokio::fs::write(&path, &content).await {
            Ok(()) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("wrote {} bytes to {path}", content.len()),
                success: true,
            },
            Err(e) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to write file '{path}': {e}"),
                success: false,
            },
        }
    })
}

// ---------------------------------------------------------------------------
// Workflow tool definitions
// ---------------------------------------------------------------------------

/// Returns the 9 workflow creation tool definitions.
#[expect(
    clippy::too_many_lines,
    reason = "9 tool definitions are inherently long"
)]
fn workflow_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "workflow_create".to_owned(),
            description: "Start a new draft workflow. Only one draft can exist at a time.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Workflow name (unique identifier)" },
                    "description": { "type": "string", "description": "Human-readable description of what the workflow does" }
                },
                "required": ["name", "description"]
            }),
        },
        ToolDefinition {
            name: "workflow_add_step".to_owned(),
            description: "Add a step to the current draft workflow.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Unique step identifier" },
                    "title": { "type": "string", "description": "Human-readable step title" },
                    "instructions": { "type": "string", "description": "Instructions for the LLM" },
                    "model_hint": { "type": "string", "enum": ["small", "medium", "large"], "description": "Model capability level" },
                    "checkpoint": { "type": "boolean", "description": "Require user approval after execution (default: false)" },
                    "requires_user_input": { "type": "boolean", "description": "Step needs user input before execution (default: false)" },
                    "tools": { "type": "array", "items": { "type": "string" }, "description": "Tools the LLM may use during this step" }
                },
                "required": ["id", "title", "instructions", "model_hint"]
            }),
        },
        ToolDefinition {
            name: "workflow_add_guard".to_owned(),
            description: "Add a guard predicate to an existing step. Guards verify step completion. Multiple guards on the same step are combined with AND logic.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "step_id": { "type": "string", "description": "The step ID to add the guard to" },
                    "predicate": {
                        "type": "string",
                        "enum": ["file_exists", "dir_exists", "file_hash_matches", "command_succeeds", "output_matches", "value_set"],
                        "description": "The guard predicate type"
                    },
                    "args": {
                        "type": "object",
                        "description": "Arguments for the predicate",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["step_id", "predicate", "args"]
            }),
        },
        ToolDefinition {
            name: "workflow_add_output".to_owned(),
            description: "Add an output descriptor to an existing step.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "step_id": { "type": "string", "description": "The step ID to add the output to" },
                    "kind": { "type": "string", "enum": ["file", "summary", "artifact"], "description": "Output type" },
                    "label": { "type": "string", "description": "Human-readable label for the output" },
                    "path": { "type": "string", "description": "File path (required for kind='file')" },
                    "value": { "type": "string", "description": "Summary value (required for kind='summary')" },
                    "description": { "type": "string", "description": "Description (required for kind='artifact')" }
                },
                "required": ["step_id", "kind", "label"]
            }),
        },
        ToolDefinition {
            name: "workflow_add_global".to_owned(),
            description: "Add or update a global template variable. Globals are available in {{var}} template expressions throughout the workflow.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Variable name" },
                    "value": { "type": "string", "description": "Variable value" }
                },
                "required": ["key", "value"]
            }),
        },
        ToolDefinition {
            name: "workflow_set_model_overrides".to_owned(),
            description: "Set model hint to model ID mappings. These override the default model for each capability level.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "small": { "type": "string", "description": "Model ID for 'small' hint" },
                    "medium": { "type": "string", "description": "Model ID for 'medium' hint" },
                    "large": { "type": "string", "description": "Model ID for 'large' hint" }
                }
            }),
        },
        ToolDefinition {
            name: "workflow_preview".to_owned(),
            description: "Return a human-readable summary of the current draft workflow for review.".to_owned(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "workflow_commit".to_owned(),
            description: "Validate the complete draft and load it as a runnable workflow. The draft is consumed on success.".to_owned(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "workflow_abort".to_owned(),
            description: "Discard the current draft workflow.".to_owned(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use nullslop_actor::MessageSink;
    use nullslop_protocol::tool::{ExecuteToolBatch, RegisterTools};
    use parking_lot::Mutex;

    use super::*;

    /// A message sink that records commands and events for test assertions.
    struct RecordingSink {
        commands: Mutex<Vec<Command>>,
        events: Mutex<Vec<Event>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                commands: Mutex::new(Vec::new()),
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<Event> {
            self.events.lock().clone()
        }

        fn take_events(&self) -> Vec<Event> {
            let mut guard = self.events.lock();
            std::mem::take(&mut guard)
        }

        fn commands(&self) -> Vec<Command> {
            self.commands.lock().clone()
        }

        fn clear(&self) {
            self.commands.lock().clear();
            self.events.lock().clear();
        }
    }

    impl MessageSink for RecordingSink {
        fn send_command(&self, command: Command) -> nullslop_actor::SendResult {
            self.commands.lock().push(command);
            Ok(())
        }

        fn send_event(&self, event: Event) -> nullslop_actor::SendResult {
            self.events.lock().push(event);
            Ok(())
        }
    }

    /// Creates a test context backed by a recording sink.
    fn test_context(sink: &std::sync::Arc<RecordingSink>) -> ActorContext {
        ActorContext::new("test-tool-orchestrator", sink.clone())
    }

    /// Creates an activated actor with a clean sink.
    fn activate() -> (
        ToolOrchestratorActor,
        std::sync::Arc<RecordingSink>,
        ActorContext,
    ) {
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let actor = ToolOrchestratorActor::activate(&mut ctx);
        sink.clear(); // Clear activation events.
        (actor, sink, ctx)
    }

    /// Creates an activated actor with a clean sink and a workflow store injected.
    fn activate_with_store(
        store_dir: &std::path::Path,
    ) -> (
        ToolOrchestratorActor,
        std::sync::Arc<RecordingSink>,
        ActorContext,
        nullslop_workflow_store::WorkflowStoreService,
    ) {
        let store = nullslop_workflow_store::FileWorkflowStore::new_in(store_dir.to_path_buf());
        let store_service =
            nullslop_workflow_store::WorkflowStoreService::new(std::sync::Arc::new(store));

        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        ctx.set_data(store_service.clone());
        let actor = ToolOrchestratorActor::activate(&mut ctx);
        sink.clear(); // Clear activation events.
        (actor, sink, ctx, store_service)
    }

    /// Sends an `ExecuteToolBatch` command and returns immediately.
    async fn send_batch(
        actor: &mut ToolOrchestratorActor,
        ctx: &ActorContext,
        session_id: SessionId,
        tool_calls: Vec<ToolCall>,
    ) {
        let cmd = Command::ExecuteToolBatch {
            payload: ExecuteToolBatch {
                session_id,
                tool_calls,
            },
        };
        actor.handle_command(&cmd, ctx).await;
    }

    /// Extracts `ToolBatchCompleted` events from a list of events.
    fn find_batch_completed(events: &[Event]) -> Vec<&ToolBatchCompleted> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::ToolBatchCompleted { payload } => Some(payload),
                _ => None,
            })
            .collect()
    }

    /// Extracts `ToolExecutionCompleted` events from a list of events.
    fn find_execution_completed(events: &[Event]) -> Vec<&ToolExecutionCompleted> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::ToolExecutionCompleted { payload } => Some(payload),
                _ => None,
            })
            .collect()
    }

    /// Extracts `LoadWorkflow` commands from a list of commands.
    fn find_load_workflow(commands: &[Command]) -> Vec<&LoadWorkflow> {
        commands
            .iter()
            .filter_map(|c| match c {
                Command::LoadWorkflow { payload } => Some(payload),
                _ => None,
            })
            .collect()
    }

    /// Creates a tool call with the given name and JSON arguments.
    fn make_call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: format!("call_{name}"),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    // --- Activation tests ---

    #[tokio::test]
    async fn activate_registers_builtin_tools() {
        // Given a fresh actor context.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);

        // When activating the actor.
        let actor = ToolOrchestratorActor::activate(&mut ctx);

        // Then the built-in tools are registered.
        assert!(actor.tools.contains_key("echo"));
        assert!(actor.tools.contains_key("get_time"));
        assert!(actor.tools.contains_key("file_read"));
        assert!(actor.tools.contains_key("file_write"));
    }

    #[tokio::test]
    async fn activate_emits_tools_registered_for_builtins() {
        // Given a fresh actor context with a recording sink.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);

        // When activating the actor.
        let _actor = ToolOrchestratorActor::activate(&mut ctx);

        // Then ToolsRegistered events were emitted for built-in and workflow tools.
        let events = sink.events();
        let tools_registered: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::ToolsRegistered { payload } => Some(payload.clone()),
                _ => None,
            })
            .collect();

        let builtin_evt = tools_registered
            .iter()
            .find(|p| p.provider == "builtin")
            .expect("expected builtin ToolsRegistered");
        assert_eq!(builtin_evt.definitions.len(), 4);

        let workflow_evt = tools_registered
            .iter()
            .find(|p| p.provider == "workflow")
            .expect("expected workflow ToolsRegistered");
        assert_eq!(workflow_evt.definitions.len(), 9);
    }

    #[tokio::test]
    async fn activate_registers_workflow_tools() {
        // Given a fresh actor context.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);

        // When activating the actor.
        let actor = ToolOrchestratorActor::activate(&mut ctx);

        // Then the 9 workflow creation tools are registered.
        let workflow_names = [
            "workflow_create",
            "workflow_add_step",
            "workflow_add_guard",
            "workflow_add_output",
            "workflow_add_global",
            "workflow_set_model_overrides",
            "workflow_preview",
            "workflow_commit",
            "workflow_abort",
        ];
        for name in workflow_names {
            assert!(
                actor.tools.contains_key(name),
                "expected workflow tool '{name}' to be registered"
            );
            match actor.get_tool(name) {
                Some(ToolRegistration::Workflow { .. }) => {}
                other => panic!("expected Workflow registration for '{name}', got {other:?}"),
            }
        }
    }

    // --- RegisterTools command tests ---

    #[tokio::test]
    async fn register_tools_stores_actor_tools() {
        // Given an activated actor.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = ToolOrchestratorActor::activate(&mut ctx);
        sink.clear();

        let definition = ToolDefinition {
            name: "web_search".to_owned(),
            description: "Search the web".to_owned(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        };

        // When registering an actor-provided tool.
        let cmd = Command::RegisterTools {
            payload: RegisterTools {
                provider: "web-actor".to_owned(),
                definitions: vec![definition],
            },
        };
        actor.handle_command(&cmd, &ctx).await;

        // Then the tool is stored in the registry.
        let reg = actor
            .get_tool("web_search")
            .expect("tool should be registered");
        match reg {
            ToolRegistration::Actor { provider, .. } => {
                assert_eq!(provider, "web-actor");
            }
            other @ (ToolRegistration::Builtin { .. } | ToolRegistration::Workflow { .. }) => {
                panic!("expected Actor registration, got {other:?}")
            }
        }
    }

    #[tokio::test]
    async fn register_tools_emits_tools_registered_event() {
        // Given an activated actor.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = ToolOrchestratorActor::activate(&mut ctx);
        sink.clear();

        let definition = ToolDefinition {
            name: "web_search".to_owned(),
            description: "Search the web".to_owned(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        };

        // When registering tools.
        let cmd = Command::RegisterTools {
            payload: RegisterTools {
                provider: "web-actor".to_owned(),
                definitions: vec![definition.clone()],
            },
        };
        actor.handle_command(&cmd, &ctx).await;

        // Then a ToolsRegistered event is emitted.
        let events = sink.events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::ToolsRegistered { payload } => {
                assert_eq!(payload.provider, "web-actor");
                assert_eq!(payload.definitions.len(), 1);
                assert_eq!(payload.definitions[0].name, "web_search");
            }
            other => panic!("expected ToolsRegistered, got {other:?}"),
        }
    }

    // --- Built-in tool execution tests ---

    #[tokio::test]
    async fn execute_builtin_echo_tool() {
        // Given an echo tool call.
        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hello world"}"#.to_owned(),
        };

        // When executing the echo tool.
        let result = execute_echo(call).await;

        // Then the result contains the echoed input.
        assert_eq!(result.tool_call_id, "call_1");
        assert_eq!(result.name, "echo");
        assert_eq!(result.content, "hello world");
        assert!(result.success);
    }

    #[tokio::test]
    async fn execute_builtin_echo_tool_returns_error_on_bad_json() {
        // Given an echo tool call with invalid JSON.
        let call = ToolCall {
            id: "call_2".to_owned(),
            name: "echo".to_owned(),
            arguments: "not json".to_owned(),
        };

        // When executing the echo tool.
        let result = execute_echo(call).await;

        // Then the result indicates failure.
        assert_eq!(result.tool_call_id, "call_2");
        assert!(!result.success);
        assert!(result.content.contains("failed to parse arguments"));
    }

    #[tokio::test]
    async fn execute_builtin_get_time_tool() {
        // Given a get_time tool call.
        let call = ToolCall {
            id: "call_3".to_owned(),
            name: "get_time".to_owned(),
            arguments: "{}".to_owned(),
        };

        // When executing the get_time tool.
        let result = execute_get_time(call).await;

        // Then the result has non-empty content.
        assert_eq!(result.tool_call_id, "call_3");
        assert!(result.success);
        assert!(!result.content.is_empty());
    }

    #[tokio::test]
    async fn execute_builtin_file_read_tool() {
        // Given a temp file with known content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "file contents here").expect("write temp file");

        let call = ToolCall {
            id: "call_4".to_owned(),
            name: "file_read".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy()
            })
            .to_string(),
        };

        // When executing the file_read tool.
        let result = execute_file_read(call).await;

        // Then the result contains the file contents.
        assert_eq!(result.tool_call_id, "call_4");
        assert!(result.success);
        assert_eq!(result.content, "file contents here");
    }

    #[tokio::test]
    async fn execute_builtin_file_read_tool_returns_error_on_missing_file() {
        // Given a file_read call for a nonexistent file.
        let call = ToolCall {
            id: "call_5".to_owned(),
            name: "file_read".to_owned(),
            arguments: serde_json::json!({
                "path": "/nonexistent/path/to/file.txt"
            })
            .to_string(),
        };

        // When executing the file_read tool.
        let result = execute_file_read(call).await;

        // Then the result indicates failure.
        assert_eq!(result.tool_call_id, "call_5");
        assert!(!result.success);
        assert!(result.content.contains("failed to read file"));
    }

    // --- Batch execution tests ---

    #[tokio::test]
    async fn execute_batch_with_single_builtin_tool() {
        // Given an activated actor.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = ToolOrchestratorActor::activate(&mut ctx);
        sink.clear();

        let session_id = SessionId::new();

        // When executing a batch with one echo call.
        let cmd = Command::ExecuteToolBatch {
            payload: ExecuteToolBatch {
                session_id: session_id.clone(),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_owned(),
                    name: "echo".to_owned(),
                    arguments: r#"{"input":"hello"}"#.to_owned(),
                }],
            },
        };
        actor.handle_command(&cmd, &ctx).await;

        // Then a ToolExecutionCompleted event arrives from the spawned task.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let events = sink.take_events();
        let completed = find_execution_completed(&events);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].result.content, "hello");
        assert!(completed[0].result.success);

        // When feeding the completion event back to the actor.
        let completion_event = Event::ToolExecutionCompleted {
            payload: ToolExecutionCompleted {
                session_id: session_id.clone(),
                result: completed[0].result.clone(),
            },
        };
        actor.handle_event(&completion_event, &ctx);

        // Then a ToolBatchCompleted event is emitted.
        let events = sink.events();
        let batch_completed = find_batch_completed(&events);
        assert_eq!(batch_completed.len(), 1);
        assert_eq!(batch_completed[0].results.len(), 1);
        assert_eq!(batch_completed[0].results[0].content, "hello");
    }

    #[tokio::test]
    async fn execute_batch_with_multiple_builtin_tools() {
        // Given an activated actor.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = ToolOrchestratorActor::activate(&mut ctx);
        sink.clear();

        let session_id = SessionId::new();

        // When executing a batch with two echo calls.
        let cmd = Command::ExecuteToolBatch {
            payload: ExecuteToolBatch {
                session_id: session_id.clone(),
                tool_calls: vec![
                    ToolCall {
                        id: "call_a".to_owned(),
                        name: "echo".to_owned(),
                        arguments: r#"{"input":"first"}"#.to_owned(),
                    },
                    ToolCall {
                        id: "call_b".to_owned(),
                        name: "echo".to_owned(),
                        arguments: r#"{"input":"second"}"#.to_owned(),
                    },
                ],
            },
        };
        actor.handle_command(&cmd, &ctx).await;

        // Then two ToolExecutionCompleted events arrive.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let events = sink.take_events();
        let completed = find_execution_completed(&events);
        assert_eq!(completed.len(), 2);

        // When feeding the first completion back.
        actor.handle_event(
            &Event::ToolExecutionCompleted {
                payload: ToolExecutionCompleted {
                    session_id: session_id.clone(),
                    result: completed[0].result.clone(),
                },
            },
            &ctx,
        );

        // Then no batch completed yet (one remaining).
        let events = sink.take_events();
        assert!(find_batch_completed(&events).is_empty());

        // When feeding the second completion back.
        actor.handle_event(
            &Event::ToolExecutionCompleted {
                payload: ToolExecutionCompleted {
                    session_id: session_id.clone(),
                    result: completed[1].result.clone(),
                },
            },
            &ctx,
        );

        // Then ToolBatchCompleted is emitted with both results.
        let events = sink.events();
        let batch_completed = find_batch_completed(&events);
        assert_eq!(batch_completed.len(), 1);
        assert_eq!(batch_completed[0].results.len(), 2);
    }

    #[tokio::test]
    async fn execute_batch_with_unknown_tool_returns_error_result() {
        // Given an activated actor.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = ToolOrchestratorActor::activate(&mut ctx);
        sink.clear();

        let session_id = SessionId::new();

        // When executing a batch with an unknown tool name.
        let cmd = Command::ExecuteToolBatch {
            payload: ExecuteToolBatch {
                session_id: session_id.clone(),
                tool_calls: vec![ToolCall {
                    id: "call_x".to_owned(),
                    name: "nonexistent_tool".to_owned(),
                    arguments: "{}".to_owned(),
                }],
            },
        };
        actor.handle_command(&cmd, &ctx).await;

        // Then a ToolExecutionCompleted event with an error is emitted synchronously.
        let events = sink.events();
        let completed = find_execution_completed(&events);
        assert_eq!(completed.len(), 1);
        assert!(!completed[0].result.success);
        assert!(completed[0].result.content.contains("unknown tool"));

        // When feeding the error result back.
        actor.handle_event(
            &Event::ToolExecutionCompleted {
                payload: ToolExecutionCompleted {
                    session_id: session_id.clone(),
                    result: completed[0].result.clone(),
                },
            },
            &ctx,
        );

        // Then ToolBatchCompleted is emitted with the error result.
        let events = sink.events();
        let batch_completed = find_batch_completed(&events);
        assert_eq!(batch_completed.len(), 1);
        assert_eq!(batch_completed[0].results.len(), 1);
        assert!(!batch_completed[0].results[0].success);
    }

    #[tokio::test]
    async fn execute_batch_with_no_tool_calls_emits_empty_batch_completed() {
        // Given an activated actor.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = ToolOrchestratorActor::activate(&mut ctx);
        sink.clear();

        let session_id = SessionId::new();

        // When executing a batch with no tool calls.
        let cmd = Command::ExecuteToolBatch {
            payload: ExecuteToolBatch {
                session_id: session_id.clone(),
                tool_calls: vec![],
            },
        };
        actor.handle_command(&cmd, &ctx).await;

        // Then an empty ToolBatchCompleted is emitted immediately.
        let events = sink.events();
        let batch_completed = find_batch_completed(&events);
        assert_eq!(batch_completed.len(), 1);
        assert!(batch_completed[0].results.is_empty());
    }

    #[tokio::test]
    async fn execute_builtin_file_write_tool() {
        // Given a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("output.txt");

        let call = ToolCall {
            id: "call_fw1".to_owned(),
            name: "file_write".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "content": "hello from file_write"
            })
            .to_string(),
        };

        // When executing the file_write tool.
        let result = execute_file_write(call).await;

        // Then the result indicates success.
        assert_eq!(result.tool_call_id, "call_fw1");
        assert!(result.success, "expected success, got: {}", result.content);
        assert!(result.content.contains("wrote 21 bytes"));

        // And the file contains the written content.
        let content = std::fs::read_to_string(&file_path).expect("read written file");
        assert_eq!(content, "hello from file_write");
    }

    #[tokio::test]
    async fn execute_builtin_file_write_tool_creates_parent_dirs() {
        // Given a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("nested").join("deep").join("file.txt");

        let call = ToolCall {
            id: "call_fw2".to_owned(),
            name: "file_write".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "content": "nested content"
            })
            .to_string(),
        };

        // When executing the file_write tool.
        let result = execute_file_write(call).await;

        // Then the result indicates success.
        assert_eq!(result.tool_call_id, "call_fw2");
        assert!(result.success, "expected success, got: {}", result.content);

        // And the file was created with parent directories.
        let content = std::fs::read_to_string(&file_path).expect("read written file");
        assert_eq!(content, "nested content");
    }

    #[tokio::test]
    async fn execute_builtin_file_write_tool_overwrites_existing_file() {
        // Given a temp file with existing content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("existing.txt");
        std::fs::write(&file_path, "old content").expect("write existing file");

        let call = ToolCall {
            id: "call_fw3".to_owned(),
            name: "file_write".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "content": "new content"
            })
            .to_string(),
        };

        // When executing the file_write tool.
        let result = execute_file_write(call).await;

        // Then the result indicates success.
        assert!(result.success);

        // And the file was overwritten.
        let content = std::fs::read_to_string(&file_path).expect("read overwritten file");
        assert_eq!(content, "new content");
    }

    #[tokio::test]
    async fn execute_builtin_file_write_tool_returns_error_on_bad_json() {
        // Given a file_write call with invalid JSON.
        let call = ToolCall {
            id: "call_fw4".to_owned(),
            name: "file_write".to_owned(),
            arguments: "not json".to_owned(),
        };

        // When executing the file_write tool.
        let result = execute_file_write(call).await;

        // Then the result indicates failure.
        assert_eq!(result.tool_call_id, "call_fw4");
        assert!(!result.success);
        assert!(result.content.contains("failed to parse arguments"));
    }

    #[tokio::test]
    async fn tool_execution_completed_for_unknown_session_is_ignored() {
        // Given an activated actor with no pending batches.
        let sink = std::sync::Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = ToolOrchestratorActor::activate(&mut ctx);
        sink.clear();

        let unknown_session = SessionId::new();

        // When receiving a ToolExecutionCompleted for an unknown session.
        let event = Event::ToolExecutionCompleted {
            payload: ToolExecutionCompleted {
                session_id: unknown_session,
                result: ToolResult {
                    tool_call_id: "call_0".to_owned(),
                    name: "echo".to_owned(),
                    content: "orphan".to_owned(),
                    success: true,
                },
            },
        };
        actor.handle_event(&event, &ctx);

        // Then no batch completed event is emitted.
        let events = sink.events();
        let batch_completed = find_batch_completed(&events);
        assert!(batch_completed.is_empty());
    }

    // --- Workflow tool tests ---

    #[tokio::test]
    async fn workflow_create_starts_draft() {
        // Given an activated actor.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When creating a workflow.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test workflow"}"#,
            )],
        )
        .await;

        // Then the batch completed immediately with a success result.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].results.len(), 1);
        assert!(batch[0].results[0].success);
        assert_eq!(batch[0].results[0].content, "Draft workflow created.");
    }

    #[tokio::test]
    async fn workflow_create_rejects_empty_name() {
        // Given an activated actor.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When creating a workflow with an empty name.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_create",
                r#"{"name":"","description":"desc"}"#,
            )],
        )
        .await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(
            batch[0].results[0]
                .content
                .contains("Failed to create workflow")
        );
    }

    #[tokio::test]
    async fn workflow_create_rejects_existing_draft() {
        // Given an activated actor with an existing draft.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"wf1","description":"first"}"#,
            )],
        )
        .await;
        sink.clear();

        // When creating a second workflow without aborting.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_create",
                r#"{"name":"wf2","description":"second"}"#,
            )],
        )
        .await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(
            batch[0].results[0]
                .content
                .contains("draft workflow already exists")
        );
    }

    #[tokio::test]
    async fn workflow_add_step_succeeds() {
        // Given an activated actor with a draft workflow.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        sink.clear();

        // When adding a step.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        ).await;

        // Then the result indicates success.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);
        assert_eq!(batch[0].results[0].content, "Step 'step-1' added.");
    }

    #[tokio::test]
    async fn workflow_add_step_rejects_duplicate_id() {
        // Given an activated actor with a draft workflow and a step.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        ).await;
        sink.clear();

        // When adding a step with the same ID.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step Two","instructions":"Do something else","model_hint":"small"}"#,
            )],
        ).await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(batch[0].results[0].content.contains("Failed to add step"));
    }

    #[tokio::test]
    async fn workflow_add_step_rejects_without_draft() {
        // Given an activated actor with no draft.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When adding a step without a draft.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        ).await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(batch[0].results[0].content.contains("no draft workflow"));
    }

    #[tokio::test]
    async fn workflow_add_guard_succeeds() {
        // Given an activated actor with a draft workflow and a step.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        ).await;
        sink.clear();

        // When adding a guard.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_add_guard",
                r#"{"step_id":"step-1","predicate":"file_exists","args":{"path":"/tmp/test.txt"}}"#,
            )],
        )
        .await;

        // Then the result indicates success.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);
        assert_eq!(batch[0].results[0].content, "Guard added to step 'step-1'.");
    }

    #[tokio::test]
    async fn workflow_add_guard_rejects_unknown_step() {
        // Given an activated actor with a draft workflow but no step with that ID.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        sink.clear();

        // When adding a guard to a non-existent step.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_add_guard",
                r#"{"step_id":"nope","predicate":"file_exists","args":{"path":"/tmp/test.txt"}}"#,
            )],
        )
        .await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(batch[0].results[0].content.contains("Failed to add guard"));
    }

    #[tokio::test]
    async fn workflow_add_guard_rejects_unknown_predicate() {
        // Given an activated actor with a draft workflow and a step.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        ).await;
        sink.clear();

        // When adding a guard with an unknown predicate type.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_add_guard",
                r#"{"step_id":"step-1","predicate":"magic_check","args":{}}"#,
            )],
        )
        .await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(
            batch[0].results[0]
                .content
                .contains("unknown predicate type")
        );
    }

    #[tokio::test]
    async fn workflow_add_output_succeeds() {
        // Given an activated actor with a draft workflow and a step.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        ).await;
        sink.clear();

        // When adding a file output.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_add_output",
                r#"{"step_id":"step-1","kind":"file","label":"Notes","path":"/tmp/notes.md"}"#,
            )],
        )
        .await;

        // Then the result indicates success.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);
        assert_eq!(
            batch[0].results[0].content,
            "Output added to step 'step-1'."
        );
    }

    #[tokio::test]
    async fn workflow_add_output_rejects_unknown_step() {
        // Given an activated actor with a draft workflow but no matching step.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        sink.clear();

        // When adding an output to a non-existent step.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_add_output",
                r#"{"step_id":"nope","kind":"file","label":"Notes","path":"/tmp/notes.md"}"#,
            )],
        )
        .await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(batch[0].results[0].content.contains("Failed to add output"));
    }

    #[tokio::test]
    async fn workflow_add_global_succeeds() {
        // Given an activated actor with a draft workflow.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        sink.clear();

        // When adding a global variable.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_add_global",
                r#"{"key":"base_dir","value":"/tmp"}"#,
            )],
        )
        .await;

        // Then the result indicates success.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);
        assert_eq!(batch[0].results[0].content, "Global 'base_dir' set.");
    }

    #[tokio::test]
    async fn workflow_set_model_overrides_succeeds() {
        // Given an activated actor with a draft workflow.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        sink.clear();

        // When setting model overrides.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call(
                "workflow_set_model_overrides",
                r#"{"small":"ollama/phi3","large":"anthropic/claude-sonnet-4"}"#,
            )],
        )
        .await;

        // Then the result indicates success.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);
        assert_eq!(batch[0].results[0].content, "2 model override(s) set.");
    }

    #[tokio::test]
    async fn workflow_preview_returns_formatted_summary() {
        // Given an activated actor with a draft workflow and a step.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test workflow"}"#,
            )],
        )
        .await;
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        ).await;
        sink.clear();

        // When previewing.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_preview", "{}")],
        )
        .await;

        // Then the result contains the workflow preview.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);
        let content = &batch[0].results[0].content;
        assert!(content.contains("Workflow: test-wf"));
        assert!(content.contains("step-1"));
    }

    #[tokio::test]
    async fn workflow_preview_rejects_without_draft() {
        // Given an activated actor with no draft.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When previewing without a draft.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_preview", "{}")],
        )
        .await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(batch[0].results[0].content.contains("no draft workflow"));
    }

    #[tokio::test]
    async fn workflow_commit_loads_workflow() {
        // Given an activated actor with a complete draft workflow.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test workflow"}"#,
            )],
        )
        .await;
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        ).await;
        sink.clear();

        // When committing the workflow.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_commit", "{}")],
        )
        .await;

        // Then the result indicates success.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);
        assert!(
            batch[0].results[0]
                .content
                .contains("committed with 1 steps")
        );

        // And a LoadWorkflow command was sent to the bus.
        let commands = sink.commands();
        let load_cmds = find_load_workflow(&commands);
        assert_eq!(load_cmds.len(), 1);
        assert_eq!(load_cmds[0].definition.name, "test-wf");
        assert_eq!(load_cmds[0].definition.steps.len(), 1);
    }

    #[tokio::test]
    async fn workflow_commit_validates_before_loading() {
        // Given an activated actor with a draft workflow but no steps.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        sink.clear();

        // When committing the incomplete workflow.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_commit", "{}")],
        )
        .await;

        // Then the result indicates validation failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(batch[0].results[0].content.contains("Validation failed"));

        // And no LoadWorkflow command was sent.
        let commands = sink.commands();
        assert!(find_load_workflow(&commands).is_empty());
    }

    #[tokio::test]
    async fn workflow_commit_clears_draft() {
        // Given an activated actor with a committed workflow.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        ).await;
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call("workflow_commit", "{}")],
        )
        .await;
        sink.clear();

        // When committing again (draft should be gone).
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_commit", "{}")],
        )
        .await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(
            batch[0].results[0]
                .content
                .contains("no draft workflow to commit")
        );
    }

    #[tokio::test]
    async fn workflow_commit_rejects_without_draft() {
        // Given an activated actor with no draft.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When committing without a draft.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_commit", "{}")],
        )
        .await;

        // Then the result indicates failure.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(
            batch[0].results[0]
                .content
                .contains("no draft workflow to commit")
        );
    }

    #[tokio::test]
    async fn workflow_abort_discards_draft() {
        // Given an activated actor with a draft workflow.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"test-wf","description":"A test"}"#,
            )],
        )
        .await;
        sink.clear();

        // When aborting the draft.
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call("workflow_abort", "{}")],
        )
        .await;

        // Then the result indicates success.
        let events = sink.take_events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);
        assert_eq!(batch[0].results[0].content, "Draft workflow discarded.");

        // And subsequent preview returns an error.
        sink.clear();
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_preview", "{}")],
        )
        .await;
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert!(!batch[0].results[0].success);
    }

    #[tokio::test]
    async fn workflow_abort_succeeds_even_without_draft() {
        // Given an activated actor with no draft.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When aborting with no draft.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_abort", "{}")],
        )
        .await;

        // Then the result indicates no draft was found.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].results[0].success);
        assert!(
            batch[0].results[0]
                .content
                .contains("no draft workflow to abort")
        );
    }

    #[tokio::test]
    async fn full_workflow_creation_flow() {
        // Given an activated actor.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When building a complete workflow through all tools.
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"video-wf","description":"Video workflow"}"#,
            )],
        )
        .await;

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"create-dir","title":"Create Dir","instructions":"Create directory","model_hint":"small","checkpoint":true,"requires_user_input":true,"tools":["shell","file_read"]}"#,
            )],
        ).await;

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_guard",
                r#"{"step_id":"create-dir","predicate":"file_exists","args":{"path":"{{dir}}/notes.md"}}"#,
            )],
        ).await;

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_output",
                r#"{"step_id":"create-dir","kind":"file","label":"Notes","path":"{{dir}}/notes.md"}"#,
            )],
        ).await;

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_global",
                r#"{"key":"dir","value":"/tmp/video"}"#,
            )],
        )
        .await;

        // Then preview shows everything.
        sink.clear();
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call("workflow_preview", "{}")],
        )
        .await;

        let preview_events = sink.take_events();
        let preview_batch = find_batch_completed(&preview_events);
        assert_eq!(preview_batch.len(), 1);
        assert!(preview_batch[0].results[0].success);
        let preview = &preview_batch[0].results[0].content;
        assert!(preview.contains("video-wf"));
        assert!(preview.contains("create-dir"));
        assert!(preview.contains("file_exists"));
        assert!(preview.contains("Notes (file)"));
        assert!(preview.contains("Globals: dir"));

        // When committing.
        sink.clear();
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_commit", "{}")],
        )
        .await;

        // Then the workflow is loaded.
        let events = sink.take_events();
        let commit_batch = find_batch_completed(&events);
        assert_eq!(commit_batch.len(), 1);
        assert!(commit_batch[0].results[0].success);

        let commands = sink.commands();
        let load_cmds = find_load_workflow(&commands);
        assert_eq!(load_cmds.len(), 1);

        let def = &load_cmds[0].definition;
        assert_eq!(def.name, "video-wf");
        assert_eq!(def.description, "Video workflow");
        assert_eq!(def.steps.len(), 1);
        assert_eq!(def.steps[0].id, "create-dir");
        assert!(def.steps[0].checkpoint);
        assert!(def.steps[0].requires_user_input);
        assert_eq!(def.globals.get("dir"), Some(&"/tmp/video".to_owned()));
    }

    // --- Mixed batch tests ---

    #[tokio::test]
    async fn batch_with_only_workflow_tools() {
        // Given an activated actor.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When sending a batch with only workflow tools.
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![
                make_call(
                    "workflow_create",
                    r#"{"name":"test-wf","description":"A test"}"#,
                ),
                make_call("workflow_preview", "{}"),
            ],
        )
        .await;

        // Then ToolBatchCompleted is emitted immediately with both results.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].results.len(), 2);

        // First result is the create result.
        assert!(batch[0].results[0].success);
        assert_eq!(batch[0].results[0].content, "Draft workflow created.");

        // Second result is the preview.
        assert!(batch[0].results[1].success);
        assert!(batch[0].results[1].content.contains("test-wf"));
    }

    #[tokio::test]
    async fn batch_with_workflow_and_regular_tools() {
        // Given an activated actor.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When sending a batch with both workflow and echo tools.
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![
                make_call(
                    "workflow_create",
                    r#"{"name":"test-wf","description":"A test"}"#,
                ),
                make_call("echo", r#"{"input":"hello"}"#),
            ],
        )
        .await;

        // Then the echo ToolExecutionCompleted arrives from the spawned task.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let events = sink.take_events();
        let completed = find_execution_completed(&events);
        assert_eq!(completed.len(), 1);

        // No batch completed yet (echo still pending).
        assert!(find_batch_completed(&events).is_empty());

        // When feeding the echo completion back.
        actor.handle_event(
            &Event::ToolExecutionCompleted {
                payload: ToolExecutionCompleted {
                    session_id: session_id.clone(),
                    result: completed[0].result.clone(),
                },
            },
            &ctx,
        );

        // Then ToolBatchCompleted has both results (workflow + echo).
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].results.len(), 2);

        // One result is the workflow create success.
        let workflow_result = batch[0]
            .results
            .iter()
            .find(|r| r.name == "workflow_create");
        assert!(workflow_result.is_some());
        assert!(workflow_result.unwrap().success);

        // One result is the echo.
        let echo_result = batch[0].results.iter().find(|r| r.name == "echo");
        assert!(echo_result.is_some());
        assert!(echo_result.unwrap().success);
    }

    // --- Workflow persistence integration tests ---

    #[tokio::test]
    async fn workflow_commit_persists_definition_to_store() {
        // Given an activated actor with a workflow store injected.
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut actor, sink, ctx, store) = activate_with_store(dir.path());
        let session_id = SessionId::new();

        // When building and committing a complete workflow.
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"persisted-wf","description":"A persisted workflow"}"#,
            )],
        )
        .await;

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        )
        .await;

        sink.clear();
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_commit", "{}")],
        )
        .await;

        // Then the commit succeeds.
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);

        // And the LoadWorkflow command was sent.
        let commands = sink.commands();
        let load_cmds = find_load_workflow(&commands);
        assert_eq!(load_cmds.len(), 1);
        assert_eq!(load_cmds[0].definition.name, "persisted-wf");

        // And the workflow definition was persisted to the store.
        let loaded = store
            .load("persisted-wf")
            .await
            .expect("load")
            .expect("should have a workflow");
        assert_eq!(loaded.name, "persisted-wf");
        assert_eq!(loaded.description, "A persisted workflow");
        assert_eq!(loaded.steps.len(), 1);
    }

    #[tokio::test]
    async fn workflow_commit_works_without_store_injected() {
        // Given an activated actor with NO workflow store injected.
        let (mut actor, sink, ctx) = activate();
        let session_id = SessionId::new();

        // When building and committing a complete workflow.
        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_create",
                r#"{"name":"no-store-wf","description":"No store workflow"}"#,
            )],
        )
        .await;

        send_batch(
            &mut actor,
            &ctx,
            session_id.clone(),
            vec![make_call(
                "workflow_add_step",
                r#"{"id":"step-1","title":"Step One","instructions":"Do something","model_hint":"small"}"#,
            )],
        )
        .await;

        sink.clear();
        send_batch(
            &mut actor,
            &ctx,
            session_id,
            vec![make_call("workflow_commit", "{}")],
        )
        .await;

        // Then the commit succeeds (no panic or error).
        let events = sink.events();
        let batch = find_batch_completed(&events);
        assert_eq!(batch.len(), 1);
        assert!(batch[0].results[0].success);

        // And the LoadWorkflow command was sent.
        let commands = sink.commands();
        let load_cmds = find_load_workflow(&commands);
        assert_eq!(load_cmds.len(), 1);
        assert_eq!(load_cmds[0].definition.name, "no-store-wf");
    }
}
