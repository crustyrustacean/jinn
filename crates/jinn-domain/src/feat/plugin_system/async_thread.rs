//! Async plugin thread — owns the background Lua state(s).
//!
//! Runs on a dedicated OS thread inside a `LocalSet`. Receives jobs from
//! a single channel (`PluginJob` enum), executes plugin hooks, and sends
//! results back through oneshot channels.
//!
//! The thread holds:
//! - A **global** `Lua` state with all `PluginKind::Global` plugins loaded.
//! - A **per-session** `Lua` state for each `SessionRegistryId`, containing
//!   the `PluginKind::Attachable` plugins that session has attached.
//!
//! Fire/Collect/SyncCollect jobs with `target_session == None` fire only
//! the global plugins. With `Some(id)`, they fire global + that session's
//! plugins (global first, then session, in deterministic order).
//!
//! The Lua state is `!Send`, so everything happens on this thread — no
//! cross-thread Lua calls.
//!
//! `ctx.request()` yields the Lua coroutine and awaits a oneshot response
//! from the tokio-side request handler. This is why the thread runs inside
//! a `LocalSet` — to allow async/await without `Send` bounds.

use serde_json::json;
use tokio::select;
use tokio_util::sync::CancellationToken;

use std::collections::HashMap;
use std::path::PathBuf;

use error_stack::{Report, ResultExt};
use mlua::Lua;
use tokio::runtime::Runtime;

use crate::SessionId;
use crate::feat::plugin_dispatch::PluginHookSite;

use super::async_handle::{PluginError, PluginJob};
use super::bindings;
use super::command::PluginCommand;
use super::loader::{PluginMeta, load_all};
use super::plugin_data::PluginData;
use super::session_registry::SessionRegistryId;
use super::sync_state::PluginHooks;

/// Callback type for handling async requests from plugins.
///
/// Called when a plugin invokes `ctx.request(name, data)`. Returns a pinned,
/// boxed future so the handler may itself `.await` async work (e.g. an LLM
/// one-shot) before resolving the awaiting Lua coroutine.
pub type RequestHandler = std::sync::Arc<
    dyn Fn(
            &str,
            &serde_json::Value,
            Option<tokio_util::sync::CancellationToken>,
        ) -> std::pin::Pin<
            std::boxed::Box<dyn std::future::Future<Output = serde_json::Value> + Send>,
        > + Send
        + Sync,
>;

/// Per-session Lua state + loaded hooks.
struct SessionState {
    /// Lua interpreter for this session.
    lua: Lua,
    /// Hooks registered per plugin for this session.
    hooks: HashMap<String, PluginHooks>,
    /// Tool definitions from attached plugins in this session.
    tools: Vec<super::tool_def::PluginToolDef>,
    /// The domain session that owns this Lua state (the "origin" session).
    /// Used to scope plugin_data correctly when tool handlers run in child sessions.
    origin_session_id: Option<SessionId>,
}

/// Thread state passed through the loop.
struct ThreadState {
    /// Global plugins state.
    global_lua: Lua,
    /// Hooks registered for global plugins.
    global_hooks: HashMap<String, PluginHooks>,
    /// Tool definitions from global plugins.
    global_tools: Vec<super::tool_def::PluginToolDef>,
    /// Per-session states keyed by registry ID.
    sessions: HashMap<SessionRegistryId, SessionState>,
    /// All discovered attachable plugins (loaded on demand).
    attachable_plugins: Vec<PluginMeta>,
    /// Shared plugin data store.
    plugin_data: PluginData,
    /// Emit channel (async).
    emit_tx: kanal::AsyncSender<PluginCommand>,
    /// Request handler.
    request_handler: RequestHandler,
    /// Shared in-flight-request registry (for ctx.cancel / cancellable ctx.request).
    in_flight: super::InFlightRequests,
}

/// Run the async plugin thread.
///
/// Blocks the calling thread forever (until the channel closes).
/// Should be called on a dedicated OS thread.
pub(crate) fn run_async_thread(
    rx: kanal::AsyncReceiver<PluginJob>,
    lua: Lua,
    hooks: HashMap<String, PluginHooks>,
    global_tools: Vec<super::tool_def::PluginToolDef>,
    all_plugins: Vec<PluginMeta>,
    plugin_data: PluginData,
    emit_tx: kanal::AsyncSender<PluginCommand>,
    request_handler: RequestHandler,
    in_flight: super::InFlightRequests,
) {
    let rt = match Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(err = %e, "failed to create runtime for async plugin thread");
            return;
        }
    };

    // Partition discovered plugins into global (already loaded) and attachable.
    // Global plugins were loaded into `lua` by PluginSystem::build; the remaining
    // attachable plugins are kept here for on-demand per-session loading.
    let attachable_plugins: Vec<PluginMeta> = all_plugins
        .into_iter()
        .filter(|m| m.kind == super::loader::PluginKind::Attachable)
        .collect();

    let state = ThreadState {
        global_lua: lua,
        global_hooks: hooks,
        global_tools,
        sessions: HashMap::new(),
        attachable_plugins,
        plugin_data,
        emit_tx,
        request_handler,
        in_flight,
    };

    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async move {
        async_thread_loop(rx, state).await;
    });
}

/// Drive the async plugin thread, executing each received [`PluginJob`] in turn.
///
/// Exits when the job channel closes.
async fn async_thread_loop(rx: kanal::AsyncReceiver<PluginJob>, mut state: ThreadState) {
    loop {
        match rx.recv().await {
            Ok(job) => execute_plugin_job(&mut state, job).await,
            Err(_) => {
                tracing::debug!("plugin thread shutting down (channel closed)");
                break;
            }
        }
    }
}

/// Execute any plugin job.
///
/// All variants respond through `tokio::sync::oneshot::Sender` with
/// `Result<T, Report<PluginError>>`. Send failures are ignored — the caller
/// may have cancelled or panicked.
#[expect(
    clippy::match_same_arms,
    reason = "Collect and SyncCollect share dispatch but originate from distinct caller paths (sync vs async); kept separate for traceability"
)]
async fn execute_plugin_job(state: &mut ThreadState, job: PluginJob) {
    match job {
        PluginJob::Fire {
            hook,
            ctx_json,
            respond_to,
            target_session,
            enabled_plugins,
        } => {
            let result =
                run_hooks_fire(state, target_session, &hook, &ctx_json, &enabled_plugins).await;
            let _ = respond_to.send(result);
        }
        PluginJob::Collect {
            hook,
            ctx_json,
            respond_to,
            target_session,
        } => {
            let result = run_hooks_collect(state, target_session, &hook, &ctx_json).await;
            let _ = respond_to.send(result);
        }
        PluginJob::SyncCollect {
            hook,
            ctx_json,
            respond_to,
            target_session,
        } => {
            let result = run_hooks_collect(state, target_session, &hook, &ctx_json).await;
            let _ = respond_to.send(result);
        }
        PluginJob::LoadSession {
            registry_id,
            plugin_names,
            origin_session_id,
            respond_to,
        } => {
            let result = load_session_plugins(state, registry_id, &plugin_names, origin_session_id);
            let _ = respond_to.send(result);
        }
        PluginJob::DestroySession { registry_id } => {
            state.sessions.remove(&registry_id);
        }
        PluginJob::ExecuteTool {
            target,
            session_id,
            plugin_name,
            tool_name,
            arguments,
            respond_to,
        } => {
            let result = execute_plugin_tool(
                state,
                target,
                &session_id,
                &plugin_name,
                &tool_name,
                &arguments,
            );
            let _ = respond_to.send(result);
        }
    }
}

/// Execute a plugin-defined tool handler.
///
/// Routes to the correct Lua state (global or per-session),
/// finds the tool handler by plugin + tool name, builds a ctx,
/// calls the handler with (ctx, arguments), returns the result string.
fn execute_plugin_tool(
    state: &mut ThreadState,
    target: Option<SessionRegistryId>,
    session_id: &SessionId,
    plugin_name: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<String, Report<PluginError>> {
    // Locate the correct Lua state and tools list.
    let (lua, tools, data_scope_id) = if let Some(id) = &target {
        let session = state.sessions.get_mut(id).ok_or_else(|| {
            Report::new(PluginError).attach(format!("no session for registry id {id:?}"))
        })?;
        let scope = session
            .origin_session_id
            .clone()
            .unwrap_or_else(|| session_id.clone());
        (&session.lua, &session.tools, scope)
    } else {
        (&state.global_lua, &state.global_tools, session_id.clone())
    };

    // Find the tool definition.
    let tool_def = tools
        .iter()
        .find(|t| t.plugin_name == plugin_name && t.name == tool_name)
        .ok_or_else(|| {
            Report::new(PluginError)
                .attach(format!("no tool '{tool_name}' for plugin '{plugin_name}'"))
        })?;

    // Build the ctx table for the handler.
    let ctx = build_async_ctx(
        lua,
        &serde_json::json!({}),
        plugin_name,
        &state.plugin_data,
        &state.emit_tx,
        &state.request_handler,
        &state.in_flight,
        Some(&data_scope_id),
    )
    .map_err(|e| Report::new(PluginError).attach(e.to_string()))
    .attach("failed to build tool handler ctx")?;

    // Convert arguments JSON to a Lua table.
    let args_value = bindings::json_to_lua_value(lua, arguments)
        .map_err(|e| Report::new(PluginError).attach(e.to_string()))?;
    let args_table = match args_value {
        mlua::Value::Table(t) => t,
        _ => lua
            .create_table()
            .map_err(|e| Report::new(PluginError).attach(e.to_string()))?,
    };

    // Get the handler function from the registry.
    let handler: mlua::Function = lua
        .registry_value(&tool_def.handler_key)
        .map_err(|e: mlua::Error| Report::new(PluginError).attach(e.to_string()))
        .attach("tool handler not found in Lua registry")?;

    // Call the handler with (ctx, args).
    let result: mlua::Value = handler
        .call::<mlua::Value>((ctx, mlua::Value::Table(args_table)))
        .map_err(|e: mlua::Error| Report::new(PluginError).attach(e.to_string()))
        .attach(format!("tool handler '{tool_name}' failed"))?;

    // Convert the return value to a string.
    let result_str = match result {
        mlua::Value::String(s) => s.to_str().map(|s| s.to_owned()).unwrap_or_default(),
        mlua::Value::Nil => String::new(),
        _ => format!("{result:?}"),
    };

    Ok(result_str)
}

/// Load attachable plugins into a new per-session Lua state.
fn load_session_plugins(
    state: &mut ThreadState,
    registry_id: SessionRegistryId,
    plugin_names: &[String],
    origin_session_id: SessionId,
) -> Result<Vec<super::tool_def::PluginToolMetadata>, Report<PluginError>> {
    // Resolve each name to a PluginMeta in the attachable set.
    let metas: Vec<PluginMeta> = plugin_names
        .iter()
        .map(|name| {
            state
                .attachable_plugins
                .iter()
                .find(|m| &m.name == name)
                .cloned()
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            Report::new(PluginError)
                .attach("one or more requested plugins not found in attachable set")
                .attach(format!("requested: {plugin_names:?}"))
        })?;

    let lua = Lua::new();
    let result = load_all(&lua, &metas);

    if result.hooks.len() != plugin_names.len() {
        let loaded_names: Vec<&str> = result.hooks.keys().map(String::as_str).collect();
        return Err(Report::new(PluginError)
            .attach("some plugins failed to load")
            .attach(format!("requested: {plugin_names:?}"))
            .attach(format!("loaded: {loaded_names:?}")));
    }
    let metadata: Vec<_> = result
        .tools
        .iter()
        .map(super::tool_def::PluginToolDef::to_metadata)
        .collect();
    state.sessions.insert(
        registry_id,
        SessionState {
            lua,
            hooks: result.hooks,
            tools: result.tools,
            origin_session_id: Some(origin_session_id),
        },
    );
    Ok(metadata)
}

/// Run global hooks + optional session hooks, discarding return values.
async fn run_hooks_fire(
    state: &mut ThreadState,
    target_session: Option<SessionRegistryId>,
    hook: &str,
    ctx_json: &serde_json::Value,
    enabled_plugins: &[String],
) -> Result<(), Report<PluginError>> {
    for (plugin_name, plugin_hooks) in &state.global_hooks {
        run_single_hook(
            &state.global_lua,
            plugin_hooks,
            hook,
            ctx_json,
            plugin_name,
            &state.plugin_data,
            &state.emit_tx,
            &state.request_handler,
            &state.in_flight,
        )
        .await?;
    }
    // Then session's plugins (filtered by enabled list).
    if let Some(id) = target_session
        && let Some(session) = state.sessions.get(&id)
    {
        for (plugin_name, plugin_hooks) in &session.hooks {
            if !enabled_plugins.is_empty() && !enabled_plugins.contains(plugin_name) {
                continue;
            }
            run_single_hook(
                &session.lua,
                plugin_hooks,
                hook,
                ctx_json,
                plugin_name,
                &state.plugin_data,
                &state.emit_tx,
                &state.request_handler,
                &state.in_flight,
            )
            .await?;
        }
    }
    Ok(())
}

/// Run global hooks + optional session hooks, collecting non-nil return values.
async fn run_hooks_collect(
    state: &mut ThreadState,
    target_session: Option<SessionRegistryId>,
    hook: &str,
    ctx_json: &serde_json::Value,
) -> Result<Vec<serde_json::Value>, Report<PluginError>> {
    let mut results = Vec::new();

    // Globals first.
    for (plugin_name, plugin_hooks) in &state.global_hooks {
        match run_single_hook(
            &state.global_lua,
            plugin_hooks,
            hook,
            ctx_json,
            plugin_name,
            &state.plugin_data,
            &state.emit_tx,
            &state.request_handler,
            &state.in_flight,
        )
        .await
        {
            Ok(Some(return_value)) => {
                match bindings::value_to_json(&state.global_lua, &return_value) {
                    Ok(json) => results.push(json),
                    Err(e) => {
                        return Err(Report::new(PluginError)
                            .attach(format!("convert return for plugin {plugin_name}: {e}")));
                    }
                }
            }
            Ok(None) => {}
            Err(report) => return Err(report),
        }
    }

    // Then session's plugins.
    if let Some(id) = target_session
        && let Some(session) = state.sessions.get(&id)
    {
        for (plugin_name, plugin_hooks) in &session.hooks {
            match run_single_hook(
                &session.lua,
                plugin_hooks,
                hook,
                ctx_json,
                plugin_name,
                &state.plugin_data,
                &state.emit_tx,
                &state.request_handler,
                &state.in_flight,
            )
            .await
            {
                Ok(Some(return_value)) => {
                    match bindings::value_to_json(&session.lua, &return_value) {
                        Ok(json) => results.push(json),
                        Err(e) => {
                            return Err(Report::new(PluginError)
                                .attach(format!("convert return for plugin {plugin_name}: {e}")));
                        }
                    }
                }
                Ok(None) => {}
                Err(report) => return Err(report),
            }
        }
    }

    Ok(results)
}

/// Run a single hook for a single plugin.
///
/// Returns `Ok(None)` if the plugin doesn't define the hook or returns nil.
/// Returns `Ok(Some(value))` if the hook returned a non-nil value.
#[expect(
    clippy::too_many_arguments,
    reason = "run-single-hook needs the full Lua+plugin+emit+handler context; bundling into a context struct is a follow-up refactor"
)]
async fn run_single_hook(
    lua: &Lua,
    plugin_hooks: &PluginHooks,
    hook: &str,
    ctx_json: &serde_json::Value,
    plugin_name: &str,
    plugin_data: &PluginData,
    emit_tx: &kanal::AsyncSender<PluginCommand>,
    request_handler: &RequestHandler,
    in_flight: &super::InFlightRequests,
) -> Result<Option<mlua::Value>, Report<PluginError>> {
    let table_opt: Option<mlua::Table> = lua
        .registry_value::<Option<mlua::Table>>(plugin_hooks.table())
        .map_err(|e: mlua::Error| Report::new(PluginError).attach(e.to_string()))
        .attach("registry lookup")?;
    let table: mlua::Table = table_opt
        .ok_or_else(|| Report::new(PluginError))
        .attach("hook table not in registry")?;

    let val: mlua::Value = table
        .get::<mlua::Value>(hook)
        .map_err(|e: mlua::Error| Report::new(PluginError).attach(e.to_string()))
        .attach(format!("hook lookup for {plugin_name}.{hook}"))?;

    let func: mlua::Function = match val {
        mlua::Value::Function(f) => f,
        _ => return Ok(None),
    };

    // Inject plugin_data into ctx JSON, scoped by session.
    let mut ctx_json = ctx_json.clone();
    let session_id = extract_session_id(&ctx_json);
    if let Some(obj) = ctx_json.as_object_mut() {
        let data = plugin_data
            .get_for_session(session_id.as_ref(), plugin_name)
            .unwrap_or(serde_json::Value::Null);
        obj.insert("plugin_data".to_owned(), data);
    }

    let ctx_table = build_async_ctx(
        lua,
        &ctx_json,
        plugin_name,
        plugin_data,
        emit_tx,
        request_handler,
        in_flight,
        session_id.as_ref(),
    )
    .map_err(|e| Report::new(PluginError).attach(e.to_string()))
    .attach("build ctx")?;

    tracing::info!(plugin = %plugin_name, %hook, "invoking async hook");
    let result: mlua::Value = match func.call_async::<mlua::Value>(ctx_table).await {
        Ok(v) => v,
        Err(e) => {
            return Err(Report::new(PluginError)
                .attach(PluginHookSite {
                    plugin: plugin_name.to_owned(),
                    hook: hook.to_owned(),
                })
                .attach(e.to_string()));
        }
    };

    match result {
        mlua::Value::Nil => Ok(None),
        other => Ok(Some(other)),
    }
}

/// Build the ctx table for an async hook call.
///
/// `ctx.request()`, `ctx.set_plugin_data()`, and `ctx.merge_plugin_data()`.
fn build_async_ctx(
    lua: &Lua,
    ctx_json: &serde_json::Value,
    plugin_name: &str,
    plugin_data: &PluginData,
    emit_tx: &kanal::AsyncSender<PluginCommand>,
    request_handler: &RequestHandler,
    in_flight: &super::InFlightRequests,
    session_id: Option<&SessionId>,
) -> Result<mlua::Table, mlua::Error> {
    let ctx = lua.create_table()?;

    // Set data fields from JSON.
    if let Some(obj) = ctx_json.as_object() {
        for (k, v) in obj {
            ctx.set(k.as_str(), bindings::json_to_lua_value(lua, v)?)?;
        }
    }

    // ctx.plugin_name — let Lua refer to itself by name (used for
    // self-targeting actions like disable_plugin).
    ctx.set("plugin_name", plugin_name)?;

    // ctx.emit(cmd, data) — fire-and-forget via channel.
    // Uses sync `Sender` (via `clone_sync()`) so it can be called from a sync
    // Lua closure. Unbounded channel => `send` is non-blocking.
    {
        let emit_tx = emit_tx.clone_sync();
        let plugin_name = plugin_name.to_owned();
        let emit_fn = lua.create_function(move |lua, (name, data): (String, mlua::Value)| {
            let json = bindings::value_to_json(lua, &data).unwrap_or_default();
            let _ = emit_tx.send(PluginCommand {
                plugin_name: plugin_name.clone(),
                name,
                data: json,
            });
            Ok(())
        })?;
        ctx.set("emit", emit_fn)?;
    }

    // ctx.request(name, data, opts?) → yields coroutine, awaits response.
    //
    // `opts` is an optional table with key `task` (string). When present, the
    // request is registered under that task name in the in-flight registry and
    // the handler future races against a cancellation token. This lets
    // `ctx.cancel(task)` abort the in-flight request from any hook (sync or
    // async) and lets `ctx.gather` await multiple requests concurrently.
    {
        let handler = request_handler.clone();
        let in_flight = in_flight.clone();
        let request_fn = lua.create_async_function(
            move |lua, (name, data, opts): (String, mlua::Value, Option<mlua::Table>)| {
                let handler = handler.clone();
                let in_flight = in_flight.clone();
                async move {
                    let json_data = bindings::value_to_json(&lua, &data).unwrap_or_default();
                    let task: Option<String> = opts
                        .as_ref()
                        .and_then(|o| o.get::<String>("task").ok());
                    let response = match task.as_deref() {
                        Some(task) => {
                            let token = in_flight.register(task);
                            select! {
                                r = handler(&name, &json_data, Some(token.clone())) => {
                                    in_flight.remove(task);
                                    r
                                }
                                _ = token.cancelled() => {
                                    json!({"ok": false, "error": "cancelled"})
                                }
                            }
                        }
                        None => handler(&name, &json_data, None).await,
                    };
                    bindings::json_to_lua_value(&lua, &response)
                }
            },
        )?;
        ctx.set("request", request_fn)?;
    }

    // ctx.set_plugin_data(value) — writes to shared DashMap.
    {
        let pd = plugin_data.clone();
        let pname = plugin_name.to_owned();
        let sid = session_id.cloned();
        let set_data_fn = lua.create_function(move |lua, value: mlua::Value| {
            let json = bindings::value_to_json(lua, &value).unwrap_or_default();
            pd.set_for_session(sid.as_ref(), &pname, json);
            Ok(())
        })?;
        ctx.set("set_plugin_data", set_data_fn)?;
    }

    // ctx.merge_plugin_data(value) — shallow-merges into the shared DashMap.
    //
    // Top-level keys in `value` overwrite the stored value's same keys;
    // other top-level keys are untouched. Lets an async hook update one
    // field (e.g. `status`) without a read-modify-write round-trip. See
    // `PluginData::merge` for merge semantics.
    {
        let pd = plugin_data.clone();
        let pname = plugin_name.to_owned();
        let sid = session_id.cloned();
        let merge_data_fn = lua.create_function(move |lua, value: mlua::Value| {
            let json = bindings::value_to_json(lua, &value).unwrap_or_default();
            pd.merge_for_session(sid.as_ref(), &pname, json);
            Ok(())
        })?;
        ctx.set("merge_plugin_data", merge_data_fn)?;
    }

    // ctx.get_plugin_data() — reads the live shared DashMap.
    //
    // Unlike the frozen `ctx.plugin_data` field (a snapshot taken at hook
    // entry), this re-reads the store on every call, so an async hook can
    // observe writes that landed after an `await` — e.g. a supersession
    // counter bumped by a concurrent fire. Returns an empty table when no
    // data is set, so Lua callers can index it directly.
    {
        let pd = plugin_data.clone();
        let pname = plugin_name.to_owned();
        let sid = session_id.cloned();
        let get_data_fn = lua.create_function(move |lua, (): ()| {
            let json = pd
                .get_for_session(sid.as_ref(), &pname)
                .unwrap_or_else(|| serde_json::json!({}));
            bindings::json_to_lua_value(lua, &json)
        })?;
        ctx.set("get_plugin_data", get_data_fn)?;
    }

    // Suppress unused warning for PathBuf import (kept for future use).
    let _: Option<PathBuf> = None;

    Ok(ctx)
}

/// Extract a `SessionId` from a hook context JSON value.
///
/// Looks for `ctx_json["session_id"]` as a string. Returns `None` for
/// global plugin hooks that don't carry a session ID.
fn extract_session_id(ctx_json: &serde_json::Value) -> Option<SessionId> {
    ctx_json
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| SessionId::from(s.to_owned()))
}
