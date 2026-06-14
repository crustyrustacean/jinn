//! Async hook tests.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_in_result,
    reason = "test code"
)]

use jinn_domain::SessionId;
use jinn_domain::feat::plugin_system::SessionPluginRegistry;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;

use jinn_domain::feat::plugin_system::{PluginCommand, PluginSystem, PluginSystemBuildResult};
use serde::Serialize;
use serde_json::json;

// ── Helpers ──────────────────────────────────────────────────────────────

fn write_plugin(dir: &Path, name: &str, lua_source: &str) {
    let plugin_dir = dir.join(name);
    std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    std::fs::write(plugin_dir.join("init.lua"), lua_source).expect("write init.lua");
}

struct TestSystem {
    captured: Arc<Mutex<Vec<PluginCommand>>>,
    async_handle: jinn_domain::feat::plugin_system::AsyncPluginHandle,
}

fn build_system(dir: &Path) -> TestSystem {
    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    // Leak the runtime — it lives for the test duration.
    // Can't drop a Runtime inside a #[tokio::test] async context.
    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));
    let PluginSystemBuildResult { async_handle, .. } = PluginSystem::build(
        dir,
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().push(cmd);
        }),
        Arc::new(|name, data, _cancel| {
            // Default request handler: echo back for "llm", null otherwise.
            let name = name.to_owned();
            let data = data.clone();
            Box::pin(async move {
                if name == "llm" {
                    json!(format!("response_to: {}", data["prompt"]))
                } else {
                    json!(null)
                }
            })
        }),
    );

    TestSystem {
        captured,
        async_handle,
    }
}

#[derive(Debug, Serialize)]
struct TurnEndCtx {
    session_id: String,
    last_assistant_message: String,
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn async_hook_fires_and_completes() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "simple",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "done",
                })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());

    let result = sys
        .async_handle
        .fire_async(
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "hello".to_owned(),
            },
        )
        .await;

    assert!(result.is_ok(), "{result:?}");

    // Give the drainer time to process.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].name, "push_chat_entry");
    assert_eq!(cmds[0].data["message"], "done");
}

#[tokio::test]
async fn async_hook_with_request_resolves() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "llm_caller",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                local reply = ctx.request("llm", { prompt = "evaluate this" })
                ctx.set_plugin_data({ verdict = reply })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());

    sys.async_handle
        .fire_async(
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "test response".to_owned(),
            },
        )
        .await
        .expect("fire");

    // Verify plugin_data was set.
    let data = sys
        .async_handle
        .get_plugin_data_for_session(&SessionId::from("s1".to_owned()), "llm_caller")
        .expect("plugin data should exist");
    assert_eq!(
        data["verdict"],
        json!("response_to: \"evaluate this\""),
        "verdict should be the LLM response"
    );
}

#[tokio::test]
async fn async_hook_with_emit_dispatches_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "emitter",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.emit("enqueue_user_message", {
                    session_id = ctx.session_id,
                    text = "follow up",
                })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());

    sys.async_handle
        .fire_async(
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "test".to_owned(),
            },
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].name, "enqueue_user_message");
    assert_eq!(cmds[0].data["text"], "follow up");
}

// ── Per-session tests ───────────────────────────────────────────────────

fn write_plugin_kind(dir: &Path, kind: &str, name: &str, lua_source: &str) {
    let plugin_dir = dir.join(kind).join(name);
    std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    std::fs::write(plugin_dir.join("init.lua"), lua_source).expect("write init.lua");
}

#[tokio::test]
async fn global_plugins_loaded_at_startup_into_shared_state() {
    // Globals in `global/` are loaded into the shared async state and fire
    // for any `fire_async` call (no session required).
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "global",
        "welcome",
        r#"
            local M = {}
            function M.on_app_started(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "global welcome",
                })
            end
            return M
        "#,
    );
    // Also drop an attachable plugin in `attachable/` to verify it does NOT
    // fire when called without a session.
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_fail",
        r#"
            local M = {}
            function M.on_app_started(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "attachable leaked",
                })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());
    sys.async_handle
        .fire_async(
            "on_app_started",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: String::new(),
            },
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    assert_eq!(cmds.len(), 1, "only global plugins should fire");
    assert_eq!(cmds[0].data["message"], "global welcome");
}

#[tokio::test]
async fn attachable_plugins_not_loaded_into_shared_state() {
    // Same setup as above, but assert by counting: only globals contribute
    // to the shared-state fan-out.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_fail",
        r#"
            local M = {}
            function M.on_app_started(ctx)
                ctx.emit("push_chat_entry", { message = "leaked" })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());
    sys.async_handle
        .fire_async(
            "on_app_started",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: String::new(),
            },
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    assert!(
        cmds.is_empty(),
        "attachable plugin should not fire globally"
    );
}

#[tokio::test]
async fn load_session_registry_creates_isolated_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_fail",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "session-only fire",
                })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(vec!["judge_fail".to_owned()], SessionId::new())
        .await
        .expect("create registry");

    sys.async_handle
        .fire_async_for_session(
            Some(result.registry_id),
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "hello".to_owned(),
            },
            vec![],
        )
        .await
        .expect("fire for session");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].data["message"], "session-only fire");
}

#[tokio::test]
async fn fire_for_session_excludes_other_sessions_plugins() {
    // Attach judge_fail to session A; fire on_turn_end for session B;
    // B has no attached plugins and should see zero emits.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_fail",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "A leaked into B",
                })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());
    let _session_a = sys
        .async_handle
        .create_session_registry(vec!["judge_fail".to_owned()], SessionId::new())
        .await
        .expect("create registry A");
    let session_b = sys
        .async_handle
        .create_session_registry(vec![], SessionId::new())
        .await
        .expect("create registry B (empty)")
        .registry_id;

    sys.async_handle
        .fire_async_for_session(
            Some(session_b),
            "on_turn_end",
            &TurnEndCtx {
                session_id: "sB".to_owned(),
                last_assistant_message: "hello".to_owned(),
            },
            vec![],
        )
        .await
        .expect("fire for B");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    assert!(
        cmds.is_empty(),
        "session A plugins must not fire for session B"
    );
}

#[tokio::test]
async fn fire_for_session_merges_global_and_session_plugins() {
    // One global + one attachable. Fire for a session that has the attachable
    // attached. Both should fire (global first, session second — order not
    // asserted here, just count and content).
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "global",
        "telemetry",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "from-global",
                })
            end
            return M
        "#,
    );
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_fail",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "from-session",
                })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(vec!["judge_fail".to_owned()], SessionId::new())
        .await
        .expect("create registry");

    sys.async_handle
        .fire_async_for_session(
            Some(result.registry_id),
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "hello".to_owned(),
            },
            vec![],
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    let messages: Vec<&str> = cmds
        .iter()
        .map(|c| c.data["message"].as_str().expect("message"))
        .collect();
    assert!(
        messages.contains(&"from-global"),
        "global plugin must fire: {messages:?}"
    );
    assert!(
        messages.contains(&"from-session"),
        "session plugin must fire: {messages:?}"
    );
}

#[tokio::test]
async fn disabled_plugin_is_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_fail",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "from-judge",
                })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(vec!["judge_fail".to_owned()], SessionId::new())
        .await
        .expect("create registry");

    // Fire with enabled_plugins set to a different plugin — judge_fail should be skipped.
    sys.async_handle
        .fire_async_for_session(
            Some(result.registry_id),
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "hello".to_owned(),
            },
            vec!["other_plugin".to_owned()],
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    let messages: Vec<&str> = cmds
        .iter()
        .map(|c| c.data["message"].as_str().expect("message"))
        .collect();

    // Then: judge_fail's hook did not fire.
    assert!(
        !messages.contains(&"from-judge"),
        "disabled plugin must not fire: {messages:?}"
    );
}

#[tokio::test]
async fn enabled_plugin_fires() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_fail",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "from-judge",
                })
            end
            return M
        "#,
    );

    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(vec!["judge_fail".to_owned()], SessionId::new())
        .await
        .expect("create registry");

    // Fire with judge_fail in enabled_plugins — it should fire.
    sys.async_handle
        .fire_async_for_session(
            Some(result.registry_id),
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "hello".to_owned(),
            },
            vec!["judge_fail".to_owned()],
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    let messages: Vec<&str> = cmds
        .iter()
        .map(|c| c.data["message"].as_str().expect("message"))
        .collect();

    // Then: judge_fail's hook fired.
    assert!(
        messages.contains(&"from-judge"),
        "enabled plugin must fire: {messages:?}"
    );
}

#[tokio::test]
async fn ctx_cancel_aborts_inflight_request() {
    // Given a plugin whose on_turn_end calls ctx.request with a task name,
    // and a request handler that parks until cancelled.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "canceler",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                local result = ctx.request("llm", { prompt = "x" }, { task = "enrich:s1" })
                ctx.set_plugin_data({ status = result.ok and "ok" or result.error })
            end
            return M
        "#,
    );

    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));

    let PluginSystemBuildResult { async_handle, .. } = PluginSystem::build(
        dir.path(),
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().push(cmd);
        }),
        Arc::new(
            |_name, _data, _cancel: Option<tokio_util::sync::CancellationToken>| {
                // Park forever — the only way out is the registry token firing,
                // which makes the select!'s cancel arm win and resume the coroutine.
                Box::pin(async move {
                    std::future::pending::<()>().await;
                    serde_json::json!(null)
                })
            },
        ),
    );

    // When firing the hook (starts the request, parks).
    let fire = tokio::spawn({
        let h = async_handle.clone();
        async move {
            h.fire_async(
                "on_turn_end",
                &TurnEndCtx {
                    session_id: "s1".to_owned(),
                    last_assistant_message: "m".to_owned(),
                },
            )
            .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then cancel the in-flight request.
    async_handle.cancel_request("enrich:s1");
    fire.await.expect("fire").expect("fire ok");

    // Give the coroutine time to resume and write plugin_data.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // The plugin_data should reflect the cancel envelope (session-scoped).
    let pd = async_handle
        .get_plugin_data_for_session(&SessionId::from("s1".to_owned()), "canceler")
        .unwrap_or_default();
    assert_eq!(
        pd["status"],
        serde_json::json!("cancelled"),
        "coroutine must resume with the cancel envelope: {pd:?}"
    );
}

#[tokio::test]
async fn gather_runs_requests_concurrently() {
    // Given a plugin that gathers 3 requests, each handler sleeping 100ms.
    // If run sequentially, total would be ~300ms; concurrently, ~100ms.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "gatherer",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                local results = ctx.gather({
                    { name = "llm", data = { id = "a" }, opts = { task = "a" } },
                    { name = "llm", data = { id = "b" }, opts = { task = "b" } },
                    { name = "llm", data = { id = "c" }, opts = { task = "c" } },
                })
                ctx.set_plugin_data({ count = #results })
            end
            return M
        "#,
    );

    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));

    let PluginSystemBuildResult { async_handle, .. } = PluginSystem::build(
        dir.path(),
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().push(cmd);
        }),
        Arc::new(|_name, _data, _cancel| {
            Box::pin(async move {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                serde_json::json!({"ok": true, "value": "done"})
            })
        }),
    );

    // When firing the hook and timing the gather.
    let start = std::time::Instant::now();
    async_handle
        .fire_async(
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "m".to_owned(),
            },
        )
        .await
        .expect("fire");
    let elapsed = start.elapsed();

    // Then the gather completed in roughly the time of ONE request (not three).
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "gather must run concurrently; elapsed {:?} suggests sequential",
        elapsed
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let pd = async_handle
        .get_plugin_data_for_session(&SessionId::from("s1".to_owned()), "gatherer")
        .unwrap_or_default();
    assert_eq!(
        pd["count"],
        serde_json::json!(3),
        "gather must return 3 results"
    );
}

#[tokio::test]
async fn cancel_one_of_two_distinct_tasks() {
    // Given a plugin that gathers two requests under distinct task names,
    // then cancels only "a". The "a" request must abort; "b" must complete.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "picker",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                local results = ctx.gather({
                    { name = "llm", data = { id = "a" }, opts = { task = "a" } },
                    { name = "llm", data = { id = "b" }, opts = { task = "b" } },
                })
                ctx.set_plugin_data({ results = results })
            end
            return M
        "#,
    );

    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));

    let PluginSystemBuildResult { async_handle, .. } = PluginSystem::build(
        dir.path(),
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().push(cmd);
        }),
        Arc::new(
            |_name, data, cancel: Option<tokio_util::sync::CancellationToken>| {
                let id = data["id"].as_str().unwrap_or("?").to_owned();
                Box::pin(async move {
                    if let Some(t) = cancel {
                        // Race sleep against cancellation.
                        tokio::select! {
                            _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {
                                serde_json::json!({"ok": true, "value": id})
                            }
                            _ = t.cancelled() => {
                                serde_json::json!({"ok": false, "error": "cancelled"})
                            }
                        }
                    } else {
                        serde_json::json!({"ok": true, "value": id})
                    }
                })
            },
        ),
    );

    let fire = tokio::spawn({
        let h = async_handle.clone();
        async move {
            h.fire_async(
                "on_turn_end",
                &TurnEndCtx {
                    session_id: "s1".to_owned(),
                    last_assistant_message: "m".to_owned(),
                },
            )
            .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Cancel only task "a"; "b" should complete normally.
    async_handle.cancel_request("a");
    fire.await.expect("fire").expect("fire ok");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let pd = async_handle
        .get_plugin_data_for_session(&SessionId::from("s1".to_owned()), "picker")
        .unwrap_or_default();
    let results = pd["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2, "gather must return both results");
    // Find which result is which by error/value.
    let a = results.iter().find(|r| r.get("error").is_some());
    let b = results
        .iter()
        .find(|r| r.get("value").and_then(|v| v.as_str()) == Some("b"));
    assert!(
        a.is_some(),
        "task 'a' must have been cancelled: {results:?}"
    );
    assert!(b.is_some(), "task 'b' must have completed: {results:?}");
}
