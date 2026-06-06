//! Capability functions — building blocks exposed to Lua scripts via ctx.
//!
//! Each `make_*` function creates a `mlua::Function` that can be registered
//! on the ctx table via [`CtxBuilder::with_function`]. Capabilities communicate
//! with the host through the [`HostRequest`] channel protocol.

use error_stack::Report;
use mlua::{Function, Lua, Table};
use tokio::sync::oneshot;

use crate::protocol::HostRequest;
use crate::registry::LuaError;

// ── llm ──────────────────────────────────────────────────────────────────

/// Creates the `ctx.llm(prompt)` async capability.
///
/// Sends a [`HostRequest::Llm`] through the host channel and awaits the response.
#[expect(
    clippy::missing_errors_doc,
    reason = "Lua bridge functions use mlua error reporting"
)]
pub fn make_llm(
    lua: &Lua,
    host_tx: kanal::Sender<HostRequest>,
    session_id: String,
    system_prompt: Option<String>,
) -> Result<Function, Report<LuaError>> {
    lua.create_async_function(move |_lua, prompt: String| {
        let host_tx = host_tx.clone();
        let session_id = session_id.clone();
        let system_prompt = system_prompt.clone();
        async move {
            let (resp_tx, resp_rx) = oneshot::channel();
            host_tx
                .send(HostRequest::Llm {
                    session_id,
                    prompt,
                    system_prompt,
                    respond_to: resp_tx,
                })
                .map_err(|e| mlua::Error::runtime(format!("send llm: {e}")))?;

            resp_rx
                .await
                .map_err(|_e| mlua::Error::runtime("llm cancelled"))?
                .map_err(|e| mlua::Error::runtime(format!("llm: {e}")))
        }
    })
    .map_err(|e| LuaError::script(format!("create llm: {e}")))
}

// ── push_user ────────────────────��───────────────────────────────────────

/// Creates the `ctx.push_user(text)` async capability.
///
/// Sends a [`HostRequest::PushUser`] through the host channel.
#[expect(
    clippy::missing_errors_doc,
    reason = "Lua bridge functions use mlua error reporting"
)]
pub fn make_push_user(
    lua: &Lua,
    host_tx: kanal::Sender<HostRequest>,
    session_id: String,
) -> Result<Function, Report<LuaError>> {
    lua.create_async_function(move |_lua, text: String| {
        let host_tx = host_tx.clone();
        let session_id = session_id.clone();
        async move {
            let (resp_tx, resp_rx) = oneshot::channel();
            host_tx
                .send(HostRequest::PushUser {
                    session_id,
                    text,
                    respond_to: resp_tx,
                })
                .map_err(|e| mlua::Error::runtime(format!("send push_user: {e}")))?;

            resp_rx
                .await
                .map_err(|_e| mlua::Error::runtime("push_user cancelled"))?
                .map_err(|e| mlua::Error::runtime(format!("push_user: {e}")))
        }
    })
    .map_err(|e| LuaError::script(format!("create push_user: {e}")))
}

// ── push_system ──────────────────────────────────────────────────────────

/// Creates the `ctx.push_system(text)` async capability.
///
/// Sends a [`HostRequest::PushSystem`] through the host channel.
#[expect(
    clippy::missing_errors_doc,
    reason = "Lua bridge functions use mlua error reporting"
)]
pub fn make_push_system(
    lua: &Lua,
    host_tx: kanal::Sender<HostRequest>,
    session_id: String,
) -> Result<Function, Report<LuaError>> {
    lua.create_async_function(move |_lua, text: String| {
        let host_tx = host_tx.clone();
        let session_id = session_id.clone();
        async move {
            let (resp_tx, resp_rx) = oneshot::channel();
            host_tx
                .send(HostRequest::PushSystem {
                    session_id,
                    text,
                    respond_to: resp_tx,
                })
                .map_err(|e| mlua::Error::runtime(format!("send push_system: {e}")))?;

            resp_rx
                .await
                .map_err(|_e| mlua::Error::runtime("push_system cancelled"))?
                .map_err(|e| mlua::Error::runtime(format!("push_system: {e}")))
        }
    })
    .map_err(|e| LuaError::script(format!("create push_system: {e}")))
}

// ── turn_off ──────────────────────��──────────────────────────────────────

/// Creates the `ctx.turn_off()` async capability.
///
/// Sends a [`HostRequest::TurnOff`] through the host channel.
#[expect(
    clippy::missing_errors_doc,
    reason = "Lua bridge functions use mlua error reporting"
)]
pub fn make_turn_off(
    lua: &Lua,
    host_tx: kanal::Sender<HostRequest>,
    workflow_id: String,
) -> Result<Function, Report<LuaError>> {
    lua.create_async_function(move |_lua, ()| {
        let host_tx = host_tx.clone();
        let workflow_id = workflow_id.clone();
        async move {
            let (resp_tx, resp_rx) = oneshot::channel();
            host_tx
                .send(HostRequest::TurnOff {
                    workflow_id,
                    respond_to: resp_tx,
                })
                .map_err(|e| mlua::Error::runtime(format!("send turn_off: {e}")))?;

            resp_rx
                .await
                .map_err(|_e| mlua::Error::runtime("turn_off cancelled"))?
                .map_err(|e| mlua::Error::runtime(format!("turn_off: {e}")))
        }
    })
    .map_err(|e| LuaError::script(format!("create turn_off: {e}")))
}

// ── gather ───────────────────────────────────────────────────────────────

/// Creates the `ctx.gather(fns)` async capability.
///
/// Takes a Lua table of functions, runs them all concurrently, and returns
/// a table of results. Does NOT go through the host channel — concurrency
/// is handled within the VM task itself.
#[expect(
    clippy::missing_errors_doc,
    reason = "Lua bridge functions use mlua error reporting"
)]
pub fn make_gather(lua: &Lua) -> Result<Function, Report<LuaError>> {
    lua.create_async_function(|lua: Lua, functions: Table| async move {
        let mut futures = Vec::new();

        for pair in functions.sequence_values::<Function>() {
            let func = pair?;
            let fut = call_fn_async(func);
            futures.push(fut);
        }

        let results = futures::future::join_all(futures).await;
        let table = lua.create_table()?;
        for result in results {
            let value = result?;
            table.push(value)?;
        }
        Ok(table)
    })
    .map_err(|e| LuaError::script(format!("create gather: {e}")))
}

/// Calls a Lua function asynchronously, returning its result.
///
/// This is a helper for `gather` — it wraps each function call in an
/// async block that can be joined concurrently.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Function must be owned by the future"
)]
fn call_fn_async(
    func: Function,
) -> impl std::future::Future<Output = Result<mlua::Value, mlua::Error>> {
    func.call_async::<mlua::Value>(())
}
