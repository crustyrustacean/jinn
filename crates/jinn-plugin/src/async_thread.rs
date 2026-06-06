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

use std::collections::HashMap;
use std::path::PathBuf;

use error_stack::{Report, ResultExt};
use mlua::Lua;
use tokio::runtime::Runtime;

use crate::async_handle::{PluginError, PluginJob};
use crate::bindings;
use crate::command::PluginCommand;
use crate::loader::{PluginMeta, load_all};
use crate::plugin_data::PluginData;
use crate::session_registry::SessionRegistryId;
use crate::sync_state::PluginHooks;

/// Callback type for handling async requests from plugins.
///
/// Called when a plugin invokes `ctx.request(name, data)`.
pub type RequestHandler =
    std::sync::Arc<dyn Fn(&str, &serde_json::Value) -> serde_json::Value + Send + Sync>;

/// Per-session Lua state + loaded hooks.
struct SessionState {
    /// Lua interpreter for this session.
    lua: Lua,
    /// Hooks registered per plugin for this session.
    hooks: HashMap<String, PluginHooks>,
}

/// Thread state passed through the loop.
struct ThreadState {
    /// Global plugins state.
    global_lua: Lua,
    /// Hooks registered for global plugins.
    global_hooks: HashMap<String, PluginHooks>,
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
}

/// Run the async plugin thread.
///
/// Blocks the calling thread forever (until the channel closes).
/// Should be called on a dedicated OS thread.
pub(crate) fn run_async_thread(
    rx: kanal::AsyncReceiver<PluginJob>,
    lua: Lua,
    hooks: HashMap<String, PluginHooks>,
    all_plugins: Vec<PluginMeta>,
    plugin_data: PluginData,
    emit_tx: kanal::AsyncSender<PluginCommand>,
    request_handler: RequestHandler,
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
        .filter(|m| m.kind == crate::loader::PluginKind::Attachable)
        .collect();

    let state = ThreadState {
        global_lua: lua,
        global_hooks: hooks,
        sessions: HashMap::new(),
        attachable_plugins,
        plugin_data,
        emit_tx,
        request_handler,
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
        } => {
            let result = run_hooks_fire(state, target_session, &hook, &ctx_json).await;
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
            respond_to,
        } => {
            let result = load_session_plugins(state, registry_id, &plugin_names);
            let _ = respond_to.send(result);
        }
        PluginJob::DestroySession { registry_id } => {
            state.sessions.remove(&registry_id);
        }
    }
}

/// Load attachable plugins into a new per-session Lua state.
fn load_session_plugins(
    state: &mut ThreadState,
    registry_id: SessionRegistryId,
    plugin_names: &[String],
) -> Result<(), Report<PluginError>> {
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
    let hooks = load_all(&lua, &metas);

    if hooks.len() != plugin_names.len() {
        let loaded_names: Vec<&str> = hooks.keys().map(String::as_str).collect();
        return Err(Report::new(PluginError)
            .attach("some plugins failed to load")
            .attach(format!("requested: {plugin_names:?}"))
            .attach(format!("loaded: {loaded_names:?}")));
    }
    state
        .sessions
        .insert(registry_id, SessionState { lua, hooks });
    Ok(())
}

/// Run global hooks + optional session hooks, discarding return values.
async fn run_hooks_fire(
    state: &mut ThreadState,
    target_session: Option<SessionRegistryId>,
    hook: &str,
    ctx_json: &serde_json::Value,
) -> Result<(), Report<PluginError>> {
    // Globals first.
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
        )
        .await?;
    }
    // Then session's plugins.
    if let Some(id) = target_session
        && let Some(session) = state.sessions.get(&id)
    {
        for (plugin_name, plugin_hooks) in &session.hooks {
            run_single_hook(
                &session.lua,
                plugin_hooks,
                hook,
                ctx_json,
                plugin_name,
                &state.plugin_data,
                &state.emit_tx,
                &state.request_handler,
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

    // Inject plugin_data into ctx JSON.
    let mut ctx_json = ctx_json.clone();
    if let Some(obj) = ctx_json.as_object_mut() {
        let data = plugin_data
            .get(plugin_name)
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
    )
    .map_err(|e| Report::new(PluginError).attach(e.to_string()))
    .attach("build ctx")?;

    let result: mlua::Value = func
        .call_async::<mlua::Value>(ctx_table)
        .await
        .map_err(|e| Report::new(PluginError).attach(e.to_string()))
        .attach(format!("hook '{plugin_name}.{hook}'"))?;

    match result {
        mlua::Value::Nil => Ok(None),
        other => Ok(Some(other)),
    }
}

/// Build the ctx table for an async hook call.
///
/// Includes data fields from `ctx_json`, `plugin_data`, `ctx.emit()`,
/// `ctx.request()`, and `ctx.set_plugin_data()`.
fn build_async_ctx(
    lua: &Lua,
    ctx_json: &serde_json::Value,
    plugin_name: &str,
    plugin_data: &PluginData,
    emit_tx: &kanal::AsyncSender<PluginCommand>,
    request_handler: &RequestHandler,
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

    // ctx.request(name, data) → yields coroutine, awaits response.
    {
        let handler = request_handler.clone();
        let request_fn =
            lua.create_async_function(move |lua, (name, data): (String, mlua::Value)| {
                let handler = handler.clone();
                async move {
                    let json_data = bindings::value_to_json(&lua, &data).unwrap_or_default();
                    let response = handler(&name, &json_data);
                    bindings::json_to_lua_value(&lua, &response)
                }
            })?;
        ctx.set("request", request_fn)?;
    }

    // ctx.set_plugin_data(value) — writes to shared DashMap.
    {
        let pd = plugin_data.clone();
        let pname = plugin_name.to_owned();
        let set_data_fn = lua.create_function(move |lua, value: mlua::Value| {
            let json = bindings::value_to_json(lua, &value).unwrap_or_default();
            pd.set(pname.clone(), json);
            Ok(())
        })?;
        ctx.set("set_plugin_data", set_data_fn)?;
    }

    // Suppress unused warning for PathBuf import (kept for future use).
    let _: Option<PathBuf> = None;

    Ok(ctx)
}
