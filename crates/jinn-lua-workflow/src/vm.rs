//! One-shot VM lifecycle — spawn a Lua task, run a script, return result.
//!
//! [`spawn_one_shot`] creates a `Lua` instance inside a `tokio::task::LocalSet`
//! (since `Lua` is `!Send`), loads the script source, extracts the `run`
//! function from the returned table, and calls it with a ctx table via
//! [`mlua::Function::call_async`].

use error_stack::Report;
use mlua::{Function, Lua, Table, Value};
use serde::Serialize;
use serde_json::Value as JsonValue;
use tokio::task::LocalSet;

use crate::protocol::HostRequest;
use crate::registry::LuaError;

/// Configuration for building the ctx table inside the VM task.
///
/// This is `Send` — it carries only data and channel senders, no `Lua` types.
/// The actual ctx table is constructed inside the `LocalSet` where `Lua` lives.
#[derive(Clone)]
pub struct CtxConfig {
    /// Serializable data to populate ctx with.
    pub data: serde_json::Value,
    /// Whether to include llm capability.
    pub llm: bool,
    /// Whether to include push_user capability.
    pub push_user: bool,
    /// Whether to include push_system capability.
    pub push_system: bool,
    /// Whether to include turn_off capability.
    pub turn_off: bool,
    /// Whether to include gather capability.
    pub gather: bool,
    /// Session ID for capabilities that need it.
    pub session_id: String,
    /// Workflow ID for turn_off capability.
    pub workflow_id: String,
    /// System prompt for llm capability.
    pub system_prompt: Option<String>,
}

impl CtxConfig {
    /// Creates a config with only data fields (no capabilities).
    pub fn data_only<S: Serialize>(data: &S) -> Self {
        Self {
            data: serde_json::to_value(data).unwrap_or_default(),
            llm: false,
            push_user: false,
            push_system: false,
            turn_off: false,
            gather: false,
            session_id: String::new(),
            workflow_id: String::new(),
            system_prompt: None,
        }
    }

    /// Enables all capabilities with the given session and workflow IDs.
    pub fn with_all_capabilities(mut self, session_id: String, workflow_id: String) -> Self {
        self.llm = true;
        self.push_user = true;
        self.push_system = true;
        self.turn_off = true;
        self.gather = true;
        self.session_id = session_id;
        self.workflow_id = workflow_id;
        self
    }

    /// Enables llm capability.
    pub fn with_llm(mut self) -> Self {
        self.llm = true;
        self
    }

    /// Enables push_user capability.
    pub fn with_push_user(mut self) -> Self {
        self.push_user = true;
        self
    }

    /// Enables push_system capability.
    pub fn with_push_system(mut self) -> Self {
        self.push_system = true;
        self
    }

    /// Enables turn_off capability.
    pub fn with_turn_off(mut self) -> Self {
        self.turn_off = true;
        self
    }

    /// Enables gather capability.
    pub fn with_gather(mut self) -> Self {
        self.gather = true;
        self
    }

    /// Sets the session ID.
    pub fn session_id(mut self, id: String) -> Self {
        self.session_id = id;
        self
    }

    /// Sets the workflow ID.
    pub fn workflow_id(mut self, id: String) -> Self {
        self.workflow_id = id;
        self
    }

    /// Sets the system prompt for llm.
    pub fn system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = Some(prompt);
        self
    }
}

/// Builds the ctx table from a [`CtxConfig`] inside the `LocalSet`.
///
/// This function runs inside the `LocalSet` where `Lua` is available.
/// It creates the ctx table, populates data fields, and registers
/// capability methods based on the config.
fn build_ctx_from_config(
    lua: &Lua,
    host_tx: kanal::Sender<HostRequest>,
    config: &CtxConfig,
) -> Result<Table, Report<LuaError>> {
    let mut builder = crate::ctx::CtxBuilder::empty(lua)?;

    // Populate data fields from the JSON value.
    if let serde_json::Value::Object(map) = &config.data {
        for (key, value) in map {
            let lua_value = crate::ctx::json_to_lua_value(lua, value)?;
            builder = builder.with_value(key, lua_value)?;
        }
    }

    // Register capabilities.
    if config.llm {
        let f = crate::capabilities::make_llm(
            lua,
            host_tx.clone(),
            config.session_id.clone(),
            config.system_prompt.clone(),
        )?;
        builder = builder.with_function("llm", f)?;
    }

    if config.push_user {
        let f = crate::capabilities::make_push_user(lua, host_tx.clone(), config.session_id.clone())?;
        builder = builder.with_function("push_user", f)?;
    }

    if config.push_system {
        let f = crate::capabilities::make_push_system(lua, host_tx.clone(), config.session_id.clone())?;
        builder = builder.with_function("push_system", f)?;
    }

    if config.turn_off {
        let f = crate::capabilities::make_turn_off(lua, host_tx.clone(), config.workflow_id.clone())?;
        builder = builder.with_function("turn_off", f)?;
    }

    if config.gather {
        let f = crate::capabilities::make_gather(lua)?;
        builder = builder.with_function("gather", f)?;
    }

    Ok(builder.build())
}

/// Spawns a one-shot Lua VM task.
///
/// Creates a new `Lua` instance on a `LocalSet` (since `Lua` is `!Send`),
/// loads the script source, extracts the `run` function, and calls it with
/// the provided ctx config.
///
/// The `Lua` instance lives entirely within the spawned thread and never
/// crosses thread boundaries.
///
/// # Arguments
///
/// * `script_source` — The Lua source code to execute.
/// * `script_name` — A name for error reporting (e.g., `"plugins/judge_fail"`).
/// * `host_tx` — Channel sender for communicating with the host handler.
/// * `ctx_config` — Configuration for building the ctx table (data + capabilities).
///
/// # Returns
///
/// A [`tokio::task::JoinHandle`] that resolves to the script's return value
/// (converted to `serde_json::Value`) or an error.
pub fn spawn_one_shot(
    script_source: String,
    script_name: String,
    host_tx: kanal::Sender<HostRequest>,
    ctx_config: CtxConfig,
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

                    // Build the ctx table from config.
                    let ctx = build_ctx_from_config(&lua, host_tx, &ctx_config).map_err(|e| {
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

                    let run_fn: Function = match table.get("run") {
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

    fn empty_config() -> CtxConfig {
        CtxConfig::data_only(&serde_json::json!({}))
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

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, empty_config());

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

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, empty_config());

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

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, empty_config());

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

        let config = CtxConfig::data_only(&serde_json::json!({ "greeting": "hello from host" }));

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, config);

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

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, empty_config());

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

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, empty_config());

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

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, empty_config());

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

        let handle = spawn_one_shot(script, "test".to_owned(), host_tx, empty_config());

        let result = handle.await.expect("task join");
        assert!(result.is_err(), "should fail for runtime error");
    }

    // ── Capability integration tests ──────────────────────────────────

    #[tokio::test]
    async fn llm_capability_sends_host_request() {
        let (host_tx, host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                    return ctx.llm("hello world")
                end
            }
        "#
        .to_owned();

        let config = empty_config()
            .with_llm()
            .session_id("session-1".to_owned());

        let handle = spawn_one_shot(script, "test-llm".to_owned(), host_tx, config);

        // Receive the host request and respond
        let req = host_rx.recv().expect("receive request");
        match req {
            HostRequest::Llm {
                prompt,
                respond_to,
                ..
            } => {
                assert_eq!(prompt, "hello world");
                respond_to
                    .send(Ok("response from llm".to_owned()))
                    .expect("respond");
            }
            other => panic!("expected Llm request, got {other}"),
        }

        let result = handle.await.expect("task join").expect("inner result");
        assert_eq!(result, serde_json::json!("response from llm"));
    }

    #[tokio::test]
    async fn push_user_capability_sends_host_request() {
        let (host_tx, host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                    ctx.push_user("judgement failed, try again")
                end
            }
        "#
        .to_owned();

        let config = empty_config()
            .with_push_user()
            .session_id("session-1".to_owned());

        let handle = spawn_one_shot(script, "test-push-user".to_owned(), host_tx, config);

        let req = host_rx.recv().expect("receive request");
        match req {
            HostRequest::PushUser {
                text,
                respond_to,
                ..
            } => {
                assert_eq!(text, "judgement failed, try again");
                respond_to.send(Ok(())).expect("respond");
            }
            other => panic!("expected PushUser request, got {other}"),
        }

        let result = handle.await.expect("task join").expect("inner result");
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn push_system_capability_sends_host_request() {
        let (host_tx, host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                    ctx.push_system("judgement passed")
                end
            }
        "#
        .to_owned();

        let config = empty_config()
            .with_push_system()
            .session_id("session-1".to_owned());

        let handle = spawn_one_shot(script, "test-push-system".to_owned(), host_tx, config);

        let req = host_rx.recv().expect("receive request");
        match req {
            HostRequest::PushSystem {
                text,
                respond_to,
                ..
            } => {
                assert_eq!(text, "judgement passed");
                respond_to.send(Ok(())).expect("respond");
            }
            other => panic!("expected PushSystem request, got {other}"),
        }

        let result = handle.await.expect("task join").expect("inner result");
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn turn_off_capability_sends_host_request() {
        let (host_tx, host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                    ctx.turn_off()
                end
            }
        "#
        .to_owned();

        let config = empty_config()
            .with_turn_off()
            .workflow_id("workflow-1".to_owned());

        let handle = spawn_one_shot(script, "test-turn-off".to_owned(), host_tx, config);

        let req = host_rx.recv().expect("receive request");
        match req {
            HostRequest::TurnOff {
                workflow_id,
                respond_to,
            } => {
                assert_eq!(workflow_id, "workflow-1");
                respond_to.send(Ok(())).expect("respond");
            }
            other => panic!("expected TurnOff request, got {other}"),
        }

        let result = handle.await.expect("task join").expect("inner result");
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn gather_runs_functions_concurrently() {
        let (host_tx, _host_rx) = kanal::unbounded::<HostRequest>();

        let script = r#"
            return {
                run = function(ctx)
                    local results = ctx.gather({
                        function() return "first" end,
                        function() return "second" end,
                        function() return "third" end,
                    })
                    -- Return the count to verify it worked
                    return #results
                end
            }
        "#
        .to_owned();

        let config = empty_config().with_gather();

        let handle = spawn_one_shot(script, "test-gather".to_owned(), host_tx, config);

        let result = handle.await.expect("task join").expect("inner result");
        // Should return 3 (the number of results)
        assert_eq!(result, serde_json::json!(3));
    }
}
