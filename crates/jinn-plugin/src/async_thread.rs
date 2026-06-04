//! Async plugin thread — owns the background Lua state.
//!
//! Runs on a dedicated OS thread inside a `LocalSet`. Receives jobs from
//! a single channel (`PluginJob` enum), executes plugin hooks, and sends
//! results back through oneshot or kanal channels.
//!
//! The Lua state is `!Send`, so everything happens here — no cross-thread
//! Lua calls.
//!
//! `ctx.request()` yields the Lua coroutine and awaits a oneshot response
//! from the tokio-side request handler. This is why the thread runs inside
//! a `LocalSet` — to allow async/await without `Send` bounds.

use std::collections::HashMap;

use mlua::Lua;
use tokio::runtime::Runtime;

use crate::async_handle::PluginJob;
use crate::bindings;
use crate::command::PluginCommand;
use crate::plugin_data::PluginData;
use crate::sync_state::PluginHooks;


/// Callback type for handling async requests from plugins.
///
/// Called when a plugin invokes `ctx.request(name, data)`.
pub type RequestHandler =
    std::sync::Arc<dyn Fn(&str, &serde_json::Value) -> serde_json::Value + Send + Sync>;

/// Run the async plugin thread.
///
/// Blocks the calling thread forever (until both channels are closed).
/// Should be called on a dedicated OS thread.
pub(crate) fn run_async_thread(
    rx: kanal::Receiver<PluginJob>,
    lua: Lua,
    hooks: HashMap<String, PluginHooks>,
    plugin_data: PluginData,
    emit_tx: kanal::Sender<PluginCommand>,
    request_handler: RequestHandler,
) {
    let rt = match Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(err = %e, "failed to create runtime for async plugin thread");
            return;
        }
    };
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async move {
        let async_rx = rx.to_async();
        async_thread_loop(async_rx, lua, hooks, plugin_data, emit_tx, request_handler)
            .await;
    });
}

async fn async_thread_loop(
    rx: kanal::AsyncReceiver<PluginJob>,
    lua: Lua,
    hooks: HashMap<String, PluginHooks>,
    plugin_data: PluginData,
    emit_tx: kanal::Sender<PluginCommand>,
    request_handler: RequestHandler,
) {
    loop {
        match rx.recv().await {
            Ok(job) => {
                execute_plugin_job(
                    &lua,
                    &hooks,
                    job,
                    &plugin_data,
                    &emit_tx,
                    &request_handler,
                )
                .await;
            }
            Err(_) => {
                tracing::debug!("plugin thread shutting down (channel closed)");
                break;
            }
        }
    }
}

/// Execute any plugin job (Fire, Collect, or SyncCollect).
async fn execute_plugin_job(
    lua: &Lua,
    hooks: &HashMap<String, PluginHooks>,
    job: PluginJob,
    plugin_data: &PluginData,
    emit_tx: &kanal::Sender<PluginCommand>,
    request_handler: &RequestHandler,
) {
    match job {
        PluginJob::Fire {
            hook,
            ctx_json,
            respond_to,
        } => {
            for (plugin_name, plugin_hooks) in hooks {
                if let Err(e) = run_single_hook(
                    lua,
                    plugin_hooks,
                    &hook,
                    &ctx_json,
                    plugin_name,
                    plugin_data,
                    emit_tx,
                    request_handler,
                )
                .await
                {
                    let _ = respond_to.send(Err(e));
                    return;
                }
            }
            let _ = respond_to.send(Ok(()));
        }
        PluginJob::Collect {
            hook,
            ctx_json,
            respond_to,
        } => {
            let results = run_all_hooks_collect(
                lua,
                hooks,
                &hook,
                &ctx_json,
                plugin_data,
                emit_tx,
                request_handler,
            )
            .await;
            let _ = respond_to.send(results);
        }
        PluginJob::SyncCollect {
            hook,
            ctx_json,
            respond_to,
        } => {
            let results = run_all_hooks_collect(
                lua,
                hooks,
                &hook,
                &ctx_json,
                plugin_data,
                emit_tx,
                request_handler,
            )
            .await;
            let _ = respond_to.send(results);
        }
    }
}

/// Run all hooks for a given name, collecting non-nil return values.
async fn run_all_hooks_collect(
    lua: &Lua,
    hooks: &HashMap<String, PluginHooks>,
    hook: &str,
    ctx_json: &serde_json::Value,
    plugin_data: &PluginData,
    emit_tx: &kanal::Sender<PluginCommand>,
    request_handler: &RequestHandler,
) -> Result<Vec<serde_json::Value>, String> {
    let mut results = Vec::new();
    for (plugin_name, plugin_hooks) in hooks {
        match run_single_hook(
            lua,
            plugin_hooks,
            hook,
            ctx_json,
            plugin_name,
            plugin_data,
            emit_tx,
            request_handler,
        )
        .await
        {
            Ok(Some(return_value)) => match bindings::value_to_json(lua, &return_value) {
                Ok(json) => results.push(json),
                Err(e) => return Err(format!("convert return: {e}")),
            },
            Ok(None) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(results)
}
/// Run a single hook for a single plugin.
///
/// Returns `Ok(None)` if the plugin doesn't define the hook or returns nil.
/// Returns `Ok(Some(value))` if the hook returned a non-nil value.
async fn run_single_hook(
    lua: &Lua,
    plugin_hooks: &PluginHooks,
    hook: &str,
    ctx_json: &serde_json::Value,
    plugin_name: &str,
    plugin_data: &PluginData,
    emit_tx: &kanal::Sender<PluginCommand>,
    request_handler: &RequestHandler,
) -> Result<Option<mlua::Value>, String> {
    let table: mlua::Table = lua
        .registry_value::<mlua::Table>(&plugin_hooks.table)
        .map_err(|e| format!("registry lookup: {e}"))?;

    let val: mlua::Value = table
        .get::<mlua::Value>(hook)
        .map_err(|e| format!("hook lookup: {e}"))?;

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
    .map_err(|e| format!("build ctx: {e}"))?;

    let result: mlua::Value = func
        .call_async::<mlua::Value>(ctx_table)
        .await
        .map_err(|e| format!("hook '{plugin_name}': {e}"))?;

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
    emit_tx: &kanal::Sender<PluginCommand>,
    request_handler: &RequestHandler,
) -> Result<mlua::Table, mlua::Error> {
    let ctx = lua.create_table()?;

    // Set data fields from JSON.
    if let Some(obj) = ctx_json.as_object() {
        for (k, v) in obj {
            ctx.set(k.as_str(), bindings::json_to_lua_value(lua, v)?)?;
        }
    }

    // ctx.emit(cmd, data) — fire-and-forget via channel.
    {
        let emit_tx = emit_tx.clone();
        let emit_fn =
            lua.create_function(move |lua, (name, data): (String, mlua::Value)| {
                let json = bindings::value_to_json(lua, &data).unwrap_or_default();
                let _ = emit_tx.send(PluginCommand { name, data: json });
                Ok(())
            })?;
        ctx.set("emit", emit_fn)?;
    }

    // ctx.request(name, data) → yields coroutine, awaits response.
    {
        let handler = request_handler.clone();
        let request_fn = lua.create_async_function(
            move |lua, (name, data): (String, mlua::Value)| {
                let handler = handler.clone();
                async move {
                    let json_data =
                        bindings::value_to_json(&lua, &data).unwrap_or_default();
                    let response = handler(&name, &json_data);
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
        let set_data_fn = lua.create_function(move |_lua, value: mlua::Value| {
            let json = bindings::value_to_json(&_lua, &value).unwrap_or_default();
            pd.set(pname.clone(), json);
            Ok(())
        })?;
        ctx.set("set_plugin_data", set_data_fn)?;
    }

    Ok(ctx)
}
