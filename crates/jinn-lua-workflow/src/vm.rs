//! One-shot VM lifecycle — spawn a Lua task, run a script, return result.
//!
//! [`spawn_one_shot`] creates a `Lua` instance inside a `tokio::task::LocalSet`
//! (since `Lua` is `!Send`), loads the script source, extracts the `run`
//! function from the returned table, and calls it with a ctx table via
//! [`mlua::Function::call_async`].

use error_stack::Report;
use mlua::{Lua, Table, Value};
use serde_json::Value as JsonValue;
use tokio::task::LocalSet;

use crate::protocol::HostRequest;
use crate::registry::LuaError;

/// Spawns a one-shot Lua VM task.
///
/// Creates a new `Lua` instance on a `LocalSet` (since `Lua` is `!Send`),
/// loads the script source, extracts the `run` function, and calls it with
/// the provided ctx table.
///
/// The `Lua` instance lives entirely within the spawned thread/task and never
/// crosses thread boundaries.
///
/// # Arguments
///
/// * `script_source` — The Lua source code to execute.
/// * `script_name` — A name for error reporting (e.g., `"plugins/judge_fail"`).
/// * `host_tx` — Channel sender for communicating with the host handler.
/// * `build_ctx` — A closure that receives the `Lua` instance and `host_tx`,
///   and returns a ctx table with data and capability methods.
///
/// # Returns
///
/// A [`tokio::task::JoinHandle`] that resolves to the script's return value
/// (converted to `serde_json::Value`) or an error.
pub fn spawn_one_shot(
    script_source: String,
    script_name: String,
    host_tx: kanal::Sender<HostRequest>,
    build_ctx: Box<
        dyn FnOnce(&Lua, kanal::Sender<HostRequest>) -> Result<Table, Report<LuaError>>
            + Send
            + 'static,
    >,
) -> tokio::task::JoinHandle<Result<JsonValue, Report<LuaError>>> {
    // We spawn a dedicated thread that runs a LocalSet.
    // This is necessary because mlua::Lua is !Send — it must stay on one thread.
    // The LocalSet allows async functions within the Lua VM to yield and resume.
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(async move {
            let local = LocalSet::new();
            local
                .run_until(async {
                    let lua = Lua::new();

                    // Build the ctx table using the provided builder.
                    let ctx = build_ctx(&lua, host_tx).map_err(|e| {
                        tracing::error!(script = %script_name, err = %e, "failed to build ctx");
                        e
                    })?;

                    // Load and execute the script to get the return table.
                    let script_table: Value = lua
                        .load(&script_source)
                        .set_name(format!("plugin/{script_name}/init.lua"))
                        .eval()
                        .map_err(|e| {
                            tracing::error!(script = %script_name, err = %e, "script load/eval failed");
                            LuaError::script(format!("script load failed: {e}"))
                        })?;

                    // Validate: must be a table with a `run` function.
                    let table: Table = match script_table {
                        Value::Table(t) => t,
                        other => {
                            return Err(LuaError::script(format!(
                                "script must return a table, got {:?}",
                                other.type_name()
                            )));
                        }
                    };

                    let run_fn: mlua::Function = match table.get("run") {
                        Ok(Value::Function(f)) => f,
                        Ok(_) => {
                            return Err(LuaError::script(
                                "script table must contain a 'run' function",
                            ));
                        }
                        Err(e) => {
                            return Err(LuaError::script(format!(
                                "error accessing 'run' field: {e}"
                            )));
                        }
                    };

                    // Call `run(ctx)` asynchronously.
                    // Async host functions yield transparently here.
                    let result: Value = run_fn
                        .call_async::<Value>(ctx)
                        .await
                        .map_err(|e| {
                            tracing::error!(script = %script_name, err = %e, "run() failed");
                            LuaError::script(format!("run() failed: {e}"))
                        })?;

                    // Convert the return value to serde_json::Value.
                    let json = value_to_json(&result);

                    Ok(json)
                })
                .await
        })
    })
    // Map the outer JoinError and flatten the Result.
    // spawn_blocking returns JoinHandle<Result<...>> so we need to handle both layers.
}

/// Converts a Lua value to `serde_json::Value` (simple conversion).
fn value_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Nil => JsonValue::Null,
        Value::Boolean(b) => JsonValue::Bool(*b),
        Value::Integer(i) => serde_json::json!(*i),
        Value::Number(n) => serde_json::json!(*n),
        Value::String(s) => JsonValue::String(s.to_string_lossy()),
        // Tables, functions, etc. — return Null for now.
        // Full table conversion will come with CtxBuilder.
        _ => JsonValue::Null,
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_in_result,
        reason = "test code, panics are acceptable"
    )]

    use super::*;

    /// Creates a simple ctx builder that provides an empty table.
    fn simple_ctx_builder() -> Box<
        dyn FnOnce(&Lua, kanal::Sender<HostRequest>) -> Result<Table, Report<LuaError>>
            + Send
            + 'static,
    > {
        Box::new(|lua: &Lua, _host_tx: kanal::Sender<HostRequest>| {
            lua.create_table()
                .map_err(|e| LuaError::script(format!("create table: {e}")))
        })
    }

    /// Creates a ctx builder that sets a data field.
    fn ctx_with_data(
        key: &str,
        value: &str,
    ) -> Box<
        dyn FnOnce(&Lua, kanal::Sender<HostRequest>) -> Result<Table, Report<LuaError>>
            + Send
            + 'static,
    > {
        let key = key.to_owned();
        let value = value.to_owned();
        Box::new(move |lua: &Lua, _host_tx: kanal::Sender<HostRequest>| {
            let table = lua
                .create_table()
                .map_err(|e| LuaError::script(format!("create table: {e}")))?;
            table
                .set(key.clone(), value.clone())
                .map_err(|e| LuaError::script(format!("set: {e}")))?;
            Ok(table)
        })
    }

    #[tokio::test]
    async fn simple_script_executes_and_returns() {
        let (host_tx, _host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                    return 42
                end
            }
        "#
        .to_owned();

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, simple_ctx_builder());

        let result = handle.await.expect("task join").expect("inner result");
        assert_eq!(result, serde_json::json!(42));
    }

    #[tokio::test]
    async fn script_returning_nil_succeeds() {
        let (host_tx, _host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                end
            }
        "#
        .to_owned();

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, simple_ctx_builder());

        let result = handle.await.expect("task join").expect("inner result");
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn script_returning_string_succeeds() {
        let (host_tx, _host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                    return "hello"
                end
            }
        "#
        .to_owned();

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, simple_ctx_builder());

        let result = handle.await.expect("task join").expect("inner result");
        assert_eq!(result, serde_json::json!("hello"));
    }

    #[tokio::test]
    async fn script_accessing_ctx_data() {
        let (host_tx, _host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                    return ctx.greeting
                end
            }
        "#
        .to_owned();

        let handle = spawn_one_shot(
            script,
            "test".to_owned(),
            host_tx,
            ctx_with_data("greeting", "hello from host"),
        );

        let result = handle.await.expect("task join").expect("inner result");
        assert_eq!(result, serde_json::json!("hello from host"));
    }

    #[tokio::test]
    async fn script_returning_non_table_fails() {
        let (host_tx, _host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return "not a table"
        "#
        .to_owned();

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, simple_ctx_builder());

        let result = handle.await.expect("task join");
        assert!(result.is_err(), "should fail for non-table return");
    }

    #[tokio::test]
    async fn script_without_run_function_fails() {
        let (host_tx, _host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return { something_else = function() end }
        "#
        .to_owned();

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, simple_ctx_builder());

        let result = handle.await.expect("task join");
        assert!(result.is_err(), "should fail without run function");
    }

    #[tokio::test]
    async fn script_with_syntax_error_fails() {
        let (host_tx, _host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return { run = function(ctx)
                this is not valid lua!!!
            end }
        "#
        .to_owned();

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, simple_ctx_builder());

        let result = handle.await.expect("task join");
        assert!(result.is_err(), "should fail for syntax error");
    }

    #[tokio::test]
    async fn script_runtime_error_fails() {
        let (host_tx, _host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                    error("something went wrong")
                end
            }
        "#
        .to_owned();

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, simple_ctx_builder());

        let result = handle.await.expect("task join");
        assert!(result.is_err(), "should fail for runtime error");
    }
}
