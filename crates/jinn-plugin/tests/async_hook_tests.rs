//! Async hook tests.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_in_result,
    reason = "test code"
)]

use jinn_core_types::PluginInstanceId;
use jinn_domain::SessionId;
use jinn_domain::feat::plugin_dispatch::SessionPluginRegistry;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;

use jinn_plugin::{PluginCommand, PluginSystem, PluginSystemBuildResult};
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
    async_handle: jinn_plugin::AsyncPluginHandle,
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
        .get_plugin_data("llm_caller")
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
        .create_session_registry(
            vec![(PluginInstanceId::new(), "judge_fail".to_owned())],
            SessionId::new(),
        )
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
        .create_session_registry(
            vec![(PluginInstanceId::new(), "judge_fail".to_owned())],
            SessionId::new(),
        )
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
        .create_session_registry(
            vec![(PluginInstanceId::new(), "judge_fail".to_owned())],
            SessionId::new(),
        )
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

    let disabled_id = PluginInstanceId::new();
    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(
            vec![(disabled_id.clone(), "judge_fail".to_owned())],
            SessionId::new(),
        )
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
            vec![PluginInstanceId::new()],
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

    let enabled_id = PluginInstanceId::new();
    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(
            vec![(enabled_id.clone(), "judge_fail".to_owned())],
            SessionId::new(),
        )
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
            vec![enabled_id.clone()],
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
async fn duplicate_plugin_instances_fire_independently() {
    // Given a session with TWO instances of the same plugin.
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
                    message = ctx.instance_id,
                })
            end
            return M
        "#,
    );

    let id_a = PluginInstanceId::new();
    let id_b = PluginInstanceId::new();
    assert_ne!(id_a, id_b, "two instances must have distinct ids");

    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(
            vec![
                (id_a.clone(), "judge_fail".to_owned()),
                (id_b.clone(), "judge_fail".to_owned()),
            ],
            SessionId::new(),
        )
        .await
        .expect("create registry");

    // When firing on_turn_end for the session (both instances enabled).
    sys.async_handle
        .fire_async_for_session(
            Some(result.registry_id),
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "hello".to_owned(),
            },
            vec![id_a.clone(), id_b.clone()],
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    let messages: Vec<&str> = cmds
        .iter()
        .map(|c| c.data["message"].as_str().expect("message"))
        .collect();

    // Then: each instance's hook fired exactly once (fire-per-instance).
    assert_eq!(
        messages.iter().filter(|m| **m == id_a.to_string()).count(),
        1,
        "instance A must fire exactly once: {messages:?}"
    );
    assert_eq!(
        messages.iter().filter(|m| **m == id_b.to_string()).count(),
        1,
        "instance B must fire exactly once: {messages:?}"
    );
}
#[tokio::test]
async fn two_instances_have_isolated_plugin_data() {
    // Given two instances of the same plugin, each writing a distinct value
    // to its own plugin_data.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "instance_writer",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.set_plugin_data({ who = ctx.instance_id })
            end
            return M
        "#,
    );

    let id_a = PluginInstanceId::new();
    let id_b = PluginInstanceId::new();
    assert_ne!(id_a, id_b);

    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(
            vec![
                (id_a.clone(), "instance_writer".to_owned()),
                (id_b.clone(), "instance_writer".to_owned()),
            ],
            SessionId::from("s1".to_owned()),
        )
        .await
        .expect("create registry");

    // When firing on_turn_end for the session (both instances enabled).
    sys.async_handle
        .fire_async_for_session(
            Some(result.registry_id),
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "hello".to_owned(),
            },
            vec![id_a.clone(), id_b.clone()],
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then: each instance's plugin_data is isolated — A reads its own id,
    // B reads its own id, and neither sees the other's write.
    let data_a = sys
        .async_handle
        .get_plugin_data_for_session(&SessionId::from("s1".to_owned()), &id_a)
        .expect("instance A data");
    let data_b = sys
        .async_handle
        .get_plugin_data_for_session(&SessionId::from("s1".to_owned()), &id_b)
        .expect("instance B data");
    assert_eq!(data_a["who"], json!(id_a.to_string()), "A reads its own id");
    assert_eq!(data_b["who"], json!(id_b.to_string()), "B reads its own id");
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
    let pd = async_handle.get_plugin_data("canceler").unwrap_or_default();
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
        "gather must run concurrently; elapsed {elapsed:?} suggests sequential",
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let pd = async_handle.get_plugin_data("gatherer").unwrap_or_default();
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
                            () = tokio::time::sleep(std::time::Duration::from_millis(150)) => {
                                serde_json::json!({"ok": true, "value": id})
                            }
                            () = t.cancelled() => {
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

    let pd = async_handle.get_plugin_data("picker").unwrap_or_default();
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

#[tokio::test]
async fn attachable_tool_executes_globally_for_descendant_session() {
    // Given an attachable plugin whose tool handler is loaded into the global
    // Lua state at startup. The tool emits a command routing back to the
    // parent session via ctx.parent_session_id.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_tool",
        r#"
            local M = {}
            M.tools = {
                {
                    name = "judgment_passed",
                    description = "Call when the response passes.",
                    scope = "attached",
                    parameters = {},
                    handler = function(ctx)
                        ctx.emit("push_chat_entry", {
                            session_id = ctx.parent_session_id,
                            message = "judged-from-global",
                        })
                    end,
                },
            }
            return M
        "#,
    );

    let sys = build_system(dir.path());

    // When executing the tool via the GLOBAL path (target: None) for a child
    // session whose id differs from the parent. This is the post-reform
    // dispatch shape — no session scope guard, no per-session registration.
    let child = SessionId::from("child-session".to_owned());
    let parent = SessionId::from("origin-session".to_owned());
    let result = sys
        .async_handle
        .execute_tool(
            None,
            child,
            Some(parent),
            "judge_tool",
            "judgment_passed",
            &json!({}),
        )
        .await;

    // Then the handler ran without rejection (no scope guard).
    assert!(
        result.is_ok(),
        "global tool execution must succeed for any calling session: {result:?}"
    );

    // And the handler emitted a command routing to the parent session id.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    let routed: Vec<&str> = cmds
        .iter()
        .filter_map(|c| c.data["session_id"].as_str())
        .collect();
    assert!(
        routed.contains(&"origin-session"),
        "handler must route via ctx.parent_session_id to origin-session: {routed:?}"
    );
}

struct AttachCtx {
    session_id: String,
    plugin_name: String,
}

impl Serialize for AttachCtx {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AttachCtx", 2)?;
        s.serialize_field("session_id", &self.session_id)?;
        s.serialize_field("plugin_name", &self.plugin_name)?;
        s.end()
    }
}

#[tokio::test]
async fn on_attach_hook_fires_per_instance_with_ctx() {
    // Given an attachable plugin with an on_attach hook that emits a command
    // recording ctx.instance_id + ctx.session_id.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_fail",
        r#"
            local M = {}
            function M.on_attach(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    instance_id = ctx.instance_id,
                })
            end
            return M
        "#,
    );

    // When creating a session registry for one instance and firing on_attach
    // scoped to that instance.
    let instance_id = PluginInstanceId::new();
    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(
            vec![(instance_id.clone(), "judge_fail".to_owned())],
            SessionId::new(),
        )
        .await
        .expect("create registry");

    sys.async_handle
        .fire_async_for_session(
            Some(result.registry_id),
            "on_attach",
            &AttachCtx {
                session_id: "s1".to_owned(),
                plugin_name: "judge_fail".to_owned(),
            },
            vec![instance_id.clone()],
        )
        .await
        .expect("fire on_attach");

    // Then the hook ran: it emitted a command carrying the instance id and
    // session id from ctx.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    assert_eq!(cmds.len(), 1, "on_attach should emit exactly once");
    assert_eq!(cmds[0].data["session_id"], "s1");
    assert_eq!(
        cmds[0].data["instance_id"],
        instance_id.to_string(),
        "ctx.instance_id must equal the fired instance id"
    );
}

#[tokio::test]
async fn on_detach_hook_fires_with_ctx() {
    // Given an attachable plugin with an on_detach hook.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "judge_fail",
        r#"
            local M = {}
            function M.on_detach(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    instance_id = ctx.instance_id,
                })
            end
            return M
        "#,
    );

    // When firing on_detach scoped to one instance (the dispatch actor fires
    // this before tearing down the registry).
    let instance_id = PluginInstanceId::new();
    let sys = build_system(dir.path());
    let result = sys
        .async_handle
        .create_session_registry(
            vec![(instance_id.clone(), "judge_fail".to_owned())],
            SessionId::new(),
        )
        .await
        .expect("create registry");

    sys.async_handle
        .fire_async_for_session(
            Some(result.registry_id),
            "on_detach",
            &AttachCtx {
                session_id: "s1".to_owned(),
                plugin_name: "judge_fail".to_owned(),
            },
            vec![instance_id.clone()],
        )
        .await
        .expect("fire on_detach");

    // Then the hook ran against the still-live registry, emitting with ctx.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let cmds = sys.captured.lock();
    assert_eq!(cmds.len(), 1, "on_detach should emit exactly once");
    assert_eq!(cmds[0].data["session_id"], "s1");
    assert_eq!(cmds[0].data["instance_id"], instance_id.to_string());
}

#[tokio::test]
async fn judge_aggregation_last_to_finish_emits_once() {
    // Given an attachable plugin implementing the aggregation protocol: on_attach
    // increments a shared count; judgment_passed posts a verdict keyed on the
    // child session (ctx.session_id) and emits ONE result only when
    // completed == count.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "panel",
        r#"
            local M = {}
            local function count_key(origin) return "judge:" .. origin .. ":count" end
            local function verdicts_key(origin) return "judge:" .. origin .. ":verdicts" end
            local function completed_key(origin) return "judge:" .. origin .. ":completed" end
            function M.on_attach(ctx)
                local k = count_key(ctx.session_id)
                ctx.set_global_data(k, (ctx.get_global_data(k) or 0) + 1)
            end
            M.tools = {
                {
                    name = "judgment_passed",
                    description = "pass",
                    scope = "attached",
                    parameters = {},
                    handler = function(ctx)
                        local origin = ctx.parent_session_id
                        local me = ctx.session_id
                        local count = ctx.get_global_data(count_key(origin)) or 0
                        local verdicts = ctx.get_global_data(verdicts_key(origin)) or {}
                        verdicts[me] = { verdict = "passed" }
                        ctx.set_global_data(verdicts_key(origin), verdicts)
                        local completed = (ctx.get_global_data(completed_key(origin)) or 0) + 1
                        ctx.set_global_data(completed_key(origin), completed)
                        if completed < count then return end
                        ctx.emit("push_chat_entry", {
                            session_id = origin,
                            kind = { transient = "merged-result" },
                        })
                    end,
                },
            }
            return M
        "#,
    );

    let sys = build_system(dir.path());

    // Attach two instances on the same origin (sets count=2 in the shared bag).
    // on_attach fires against a session registry; the bag is shared with the
    // global tool-execution Lua state.
    let origin = SessionId::from("origin".to_owned());
    let inst_a = PluginInstanceId::new();
    let inst_b = PluginInstanceId::new();
    for inst in [&inst_a, &inst_b] {
        let reg = sys
            .async_handle
            .create_session_registry(vec![(inst.clone(), "panel".to_owned())], SessionId::new())
            .await
            .expect("create registry");
        sys.async_handle
            .fire_async_for_session(
                Some(reg.registry_id),
                "on_attach",
                &AttachCtx {
                    session_id: origin.to_string(),
                    plugin_name: "panel".to_owned(),
                },
                vec![inst.clone()],
            )
            .await
            .expect("fire on_attach");
    }
    // Give the async hook fires time to land.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // First child posts its verdict. completed=1 < count=2 → no emit yet.
    sys.captured.lock().clear();
    sys.async_handle
        .execute_tool(
            None,
            SessionId::from("child-a".to_owned()),
            Some(origin.clone()),
            "panel",
            "judgment_passed",
            &json!({}),
        )
        .await
        .expect("execute child-a verdict");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        sys.captured.lock().len(),
        0,
        "first verdict must not emit while completed < count"
    );

    // Second child posts — now completed=2 == count=2 → exactly one emit.
    sys.async_handle
        .execute_tool(
            None,
            SessionId::from("child-b".to_owned()),
            Some(origin.clone()),
            "panel",
            "judgment_passed",
            &json!({}),
        )
        .await
        .expect("execute child-b verdict");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then exactly one result was emitted to the origin.
    let cmds = sys.captured.lock();
    let emits: Vec<_> = cmds
        .iter()
        .filter(|c| c.name == "push_chat_entry")
        .collect();
    assert_eq!(
        emits.len(),
        1,
        "exactly one merged result must be emitted (last-to-finish aggregates)"
    );
    assert_eq!(emits[0].data["session_id"], "origin");
}

#[tokio::test]
async fn judge_aggregation_single_instance_emits_directly() {
    // Given the aggregation protocol with a SINGLE instance (count=1). The
    // single verdict immediately satisfies completed == count, so it emits
    // directly — identical to pre-aggregation behavior.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "panel",
        r#"
            local M = {}
            local function count_key(o) return "judge:" .. o .. ":count" end
            local function verdicts_key(o) return "judge:" .. o .. ":verdicts" end
            local function completed_key(o) return "judge:" .. o .. ":completed" end
            function M.on_attach(ctx)
                local k = count_key(ctx.session_id)
                ctx.set_global_data(k, (ctx.get_global_data(k) or 0) + 1)
            end
            M.tools = {
                {
                    name = "judgment_passed",
                    description = "pass",
                    scope = "attached",
                    parameters = {},
                    handler = function(ctx)
                        local origin = ctx.parent_session_id
                        local me = ctx.session_id
                        local count = ctx.get_global_data(count_key(origin)) or 0
                        local verdicts = ctx.get_global_data(verdicts_key(origin)) or {}
                        verdicts[me] = { verdict = "passed" }
                        ctx.set_global_data(verdicts_key(origin), verdicts)
                        local completed = (ctx.get_global_data(completed_key(origin)) or 0) + 1
                        ctx.set_global_data(completed_key(origin), completed)
                        if completed < count then return end
                        ctx.emit("push_chat_entry", {
                            session_id = origin,
                            kind = { transient = "result" },
                        })
                    end,
                },
            }
            return M
        "#,
    );

    let sys = build_system(dir.path());

    // Attach ONE instance → count=1.
    let origin = SessionId::from("origin".to_owned());
    let inst = PluginInstanceId::new();
    let reg = sys
        .async_handle
        .create_session_registry(vec![(inst.clone(), "panel".to_owned())], SessionId::new())
        .await
        .expect("create registry");
    sys.async_handle
        .fire_async_for_session(
            Some(reg.registry_id),
            "on_attach",
            &AttachCtx {
                session_id: origin.to_string(),
                plugin_name: "panel".to_owned(),
            },
            vec![inst.clone()],
        )
        .await
        .expect("fire on_attach");
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // When the single child posts its verdict.
    sys.async_handle
        .execute_tool(
            None,
            SessionId::from("child".to_owned()),
            Some(origin.clone()),
            "panel",
            "judgment_passed",
            &json!({}),
        )
        .await
        .expect("execute verdict");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then it emits directly — no waiting for siblings.
    let cmds = sys.captured.lock();
    assert_eq!(
        cmds.iter().filter(|c| c.name == "push_chat_entry").count(),
        1,
        "single instance must emit immediately (backward compatible)"
    );
}

#[tokio::test]
async fn judge_aggregation_all_pass_disables_every_instance() {
    // Given two judge instances where all-must-finish aggregation disables EVERY
    // instance on pass (not just the aggregator). The test plugin mirrors the
    // judge protocol: on_attach records the instance id in a shared set; the
    // last verdict reads that set and emits disable_plugin per instance.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "panel",
        r#"
            local M = {}
            local function count_key(o) return "judge:" .. o .. ":count" end
            local function verdicts_key(o) return "judge:" .. o .. ":verdicts" end
            local function completed_key(o) return "judge:" .. o .. ":completed" end
            local function instances_key(o) return "judge:" .. o .. ":instances" end
            function M.on_attach(ctx)
                local k = count_key(ctx.session_id)
                ctx.set_global_data(k, (ctx.get_global_data(k) or 0) + 1)
                local ikey = instances_key(ctx.session_id)
                local instances = ctx.get_global_data(ikey) or {}
                instances[ctx.instance_id] = true
                ctx.set_global_data(ikey, instances)
            end
            M.tools = {
                {
                    name = "judgment_passed",
                    description = "pass",
                    scope = "attached",
                    parameters = {},
                    handler = function(ctx)
                        local origin = ctx.parent_session_id
                        local me = ctx.session_id
                        local count = ctx.get_global_data(count_key(origin)) or 0
                        local verdicts = ctx.get_global_data(verdicts_key(origin)) or {}
                        verdicts[me] = { verdict = "passed" }
                        ctx.set_global_data(verdicts_key(origin), verdicts)
                        local completed = (ctx.get_global_data(completed_key(origin)) or 0) + 1
                        ctx.set_global_data(completed_key(origin), completed)
                        if completed < count then return end
                        ctx.emit("push_chat_entry", {
                            session_id = origin,
                            kind = { transient = "result" },
                        })
                        local instances = ctx.get_global_data(instances_key(origin)) or {}
                        for instance_id, _ in pairs(instances) do
                            ctx.emit("disable_plugin", {
                                session_id = origin,
                                plugin_name = ctx.plugin_name,
                                instance_id = instance_id,
                            })
                        end
                    end,
                },
            }
            return M
        "#,
    );

    let sys = build_system(dir.path());
    let origin = SessionId::from("origin".to_owned());
    let inst_a = PluginInstanceId::new();
    let inst_b = PluginInstanceId::new();
    for inst in [&inst_a, &inst_b] {
        let reg = sys
            .async_handle
            .create_session_registry(vec![(inst.clone(), "panel".to_owned())], SessionId::new())
            .await
            .expect("create registry");
        sys.async_handle
            .fire_async_for_session(
                Some(reg.registry_id),
                "on_attach",
                &AttachCtx {
                    session_id: origin.to_string(),
                    plugin_name: "panel".to_owned(),
                },
                vec![inst.clone()],
            )
            .await
            .expect("fire on_attach");
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // When both children post passing verdicts.
    for child in ["child-a", "child-b"] {
        sys.async_handle
            .execute_tool(
                None,
                SessionId::from(child.to_owned()),
                Some(origin.clone()),
                "panel",
                "judgment_passed",
                &json!({}),
            )
            .await
            .expect("execute verdict");
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then disable_plugin was emitted for BOTH instance ids (all-pass → disable all).
    let cmds = sys.captured.lock();
    let disable_ids: Vec<String> = cmds
        .iter()
        .filter(|c| c.name == "disable_plugin")
        .map(|c| c.data["instance_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        disable_ids.len(),
        2,
        "all-pass must disable every participating instance"
    );
    assert!(
        disable_ids.contains(&inst_a.to_string()),
        "instance a disabled: {disable_ids:?}"
    );
    assert!(
        disable_ids.contains(&inst_b.to_string()),
        "instance b disabled: {disable_ids:?}"
    );
}

#[tokio::test]
async fn judge_aggregation_any_fail_reenables_every_instance() {
    // Given two judge instances where ANY failure re-enables ALL instances
    // (re-activate on failure). The test plugin emits enable_plugin per
    // instance in the shared set when any verdict is "failed".
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "panel",
        r#"
            local M = {}
            local function count_key(o) return "judge:" .. o .. ":count" end
            local function verdicts_key(o) return "judge:" .. o .. ":verdicts" end
            local function completed_key(o) return "judge:" .. o .. ":completed" end
            local function instances_key(o) return "judge:" .. o .. ":instances" end
            function M.on_attach(ctx)
                local k = count_key(ctx.session_id)
                ctx.set_global_data(k, (ctx.get_global_data(k) or 0) + 1)
                local ikey = instances_key(ctx.session_id)
                local instances = ctx.get_global_data(ikey) or {}
                instances[ctx.instance_id] = true
                ctx.set_global_data(ikey, instances)
            end
            M.tools = {
                {
                    name = "judgment_failed",
                    description = "fail",
                    scope = "attached",
                    parameters = {
                        { name = "message", type = "string", description = "why" },
                    },
                    handler = function(ctx, args)
                        local origin = ctx.parent_session_id
                        local me = ctx.session_id
                        local count = ctx.get_global_data(count_key(origin)) or 0
                        local verdicts = ctx.get_global_data(verdicts_key(origin)) or {}
                        verdicts[me] = { verdict = "failed", message = args.message }
                        ctx.set_global_data(verdicts_key(origin), verdicts)
                        local completed = (ctx.get_global_data(completed_key(origin)) or 0) + 1
                        ctx.set_global_data(completed_key(origin), completed)
                        if completed < count then return end
                        ctx.emit("enqueue_user_message", {
                            session_id = origin,
                            text = "failed",
                        })
                        local instances = ctx.get_global_data(instances_key(origin)) or {}
                        for instance_id, _ in pairs(instances) do
                            ctx.emit("enable_plugin", {
                                session_id = origin,
                                plugin_name = ctx.plugin_name,
                                instance_id = instance_id,
                            })
                        end
                    end,
                },
            }
            return M
        "#,
    );

    let sys = build_system(dir.path());
    let origin = SessionId::from("origin".to_owned());
    let inst_a = PluginInstanceId::new();
    let inst_b = PluginInstanceId::new();
    for inst in [&inst_a, &inst_b] {
        let reg = sys
            .async_handle
            .create_session_registry(vec![(inst.clone(), "panel".to_owned())], SessionId::new())
            .await
            .expect("create registry");
        sys.async_handle
            .fire_async_for_session(
                Some(reg.registry_id),
                "on_attach",
                &AttachCtx {
                    session_id: origin.to_string(),
                    plugin_name: "panel".to_owned(),
                },
                vec![inst.clone()],
            )
            .await
            .expect("fire on_attach");
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // When both children post failing verdicts.
    for child in ["child-a", "child-b"] {
        sys.async_handle
            .execute_tool(
                None,
                SessionId::from(child.to_owned()),
                Some(origin.clone()),
                "panel",
                "judgment_failed",
                &json!({ "message": "bad" }),
            )
            .await
            .expect("execute verdict");
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then enable_plugin was emitted for BOTH instance ids (any-fail → re-enable all).
    let cmds = sys.captured.lock();
    let enable_ids: Vec<String> = cmds
        .iter()
        .filter(|c| c.name == "enable_plugin")
        .map(|c| c.data["instance_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        enable_ids.len(),
        2,
        "any-fail must re-enable every participating instance"
    );
    assert!(
        enable_ids.contains(&inst_a.to_string()),
        "instance a re-enabled: {enable_ids:?}"
    );
    assert!(
        enable_ids.contains(&inst_b.to_string()),
        "instance b re-enabled: {enable_ids:?}"
    );
}

#[tokio::test]
async fn judge_aggregation_single_instance_backward_compat_disables_itself() {
    // Given a SINGLE judge instance (count=1). On pass it disables itself — the
    // only instance — preserving the pre-aggregation one-shot behavior.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(
        dir.path(),
        "attachable",
        "panel",
        r#"
            local M = {}
            local function count_key(o) return "judge:" .. o .. ":count" end
            local function verdicts_key(o) return "judge:" .. o .. ":verdicts" end
            local function completed_key(o) return "judge:" .. o .. ":completed" end
            local function instances_key(o) return "judge:" .. o .. ":instances" end
            function M.on_attach(ctx)
                local k = count_key(ctx.session_id)
                ctx.set_global_data(k, (ctx.get_global_data(k) or 0) + 1)
                local ikey = instances_key(ctx.session_id)
                local instances = ctx.get_global_data(ikey) or {}
                instances[ctx.instance_id] = true
                ctx.set_global_data(ikey, instances)
            end
            M.tools = {
                {
                    name = "judgment_passed",
                    description = "pass",
                    scope = "attached",
                    parameters = {},
                    handler = function(ctx)
                        local origin = ctx.parent_session_id
                        local me = ctx.session_id
                        local count = ctx.get_global_data(count_key(origin)) or 0
                        local verdicts = ctx.get_global_data(verdicts_key(origin)) or {}
                        verdicts[me] = { verdict = "passed" }
                        ctx.set_global_data(verdicts_key(origin), verdicts)
                        local completed = (ctx.get_global_data(completed_key(origin)) or 0) + 1
                        ctx.set_global_data(completed_key(origin), completed)
                        if completed < count then return end
                        ctx.emit("push_chat_entry", {
                            session_id = origin,
                            kind = { transient = "result" },
                        })
                        local instances = ctx.get_global_data(instances_key(origin)) or {}
                        for instance_id, _ in pairs(instances) do
                            ctx.emit("disable_plugin", {
                                session_id = origin,
                                plugin_name = ctx.plugin_name,
                                instance_id = instance_id,
                            })
                        end
                    end,
                },
            }
            return M
        "#,
    );

    let sys = build_system(dir.path());
    let origin = SessionId::from("origin".to_owned());
    let inst = PluginInstanceId::new();
    let reg = sys
        .async_handle
        .create_session_registry(vec![(inst.clone(), "panel".to_owned())], SessionId::new())
        .await
        .expect("create registry");
    sys.async_handle
        .fire_async_for_session(
            Some(reg.registry_id),
            "on_attach",
            &AttachCtx {
                session_id: origin.to_string(),
                plugin_name: "panel".to_owned(),
            },
            vec![inst.clone()],
        )
        .await
        .expect("fire on_attach");
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // When the single child posts its verdict.
    sys.async_handle
        .execute_tool(
            None,
            SessionId::from("child".to_owned()),
            Some(origin.clone()),
            "panel",
            "judgment_passed",
            &json!({}),
        )
        .await
        .expect("execute verdict");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then it disables exactly itself (the only instance).
    let cmds = sys.captured.lock();
    let disable_ids: Vec<String> = cmds
        .iter()
        .filter(|c| c.name == "disable_plugin")
        .map(|c| c.data["instance_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        disable_ids,
        vec![inst.to_string()],
        "single instance disables itself"
    );
}

// ─── Majority-vote aggregation ────────────────────────────────────────────

/// Inline plugin mirroring the real judge's majority aggregation: strict
/// majority pass wins; otherwise (fail-majority or tie) fail, concatenating
/// only the failed verdicts' messages.
const MAJORITY_PANEL_LUA: &str = r#"
    local M = {}
    local function count_key(o) return "judge:" .. o .. ":count" end
    local function verdicts_key(o) return "judge:" .. o .. ":verdicts" end
    local function completed_key(o) return "judge:" .. o .. ":completed" end
    local function instances_key(o) return "judge:" .. o .. ":instances" end
    local function record(ctx, verdict, message)
        local origin = ctx.parent_session_id
        local me = ctx.session_id
        local count = ctx.get_global_data(count_key(origin)) or 0
        local verdicts = ctx.get_global_data(verdicts_key(origin)) or {}
        verdicts[me] = { verdict = verdict, message = message }
        ctx.set_global_data(verdicts_key(origin), verdicts)
        local completed = (ctx.get_global_data(completed_key(origin)) or 0) + 1
        ctx.set_global_data(completed_key(origin), completed)
        if completed < count then return end
        local pass_count = 0
        local fail_count = 0
        local fail_parts = {}
        for _id, v in pairs(verdicts) do
            if v.verdict == "passed" then pass_count = pass_count + 1
            elseif v.verdict == "failed" then
                fail_count = fail_count + 1
                table.insert(fail_parts, v.message or "(no reason given)")
            end
        end
        local passed = pass_count > fail_count
        if passed then
            ctx.emit("push_chat_entry", { session_id = origin, kind = { transient = "pass" } })
        else
            ctx.emit("enqueue_user_message", { session_id = origin, text = "fail:" .. table.concat(fail_parts, ";") })
        end
    end
    function M.on_attach(ctx)
        local k = count_key(ctx.session_id)
        ctx.set_global_data(k, (ctx.get_global_data(k) or 0) + 1)
        local ikey = instances_key(ctx.session_id)
        local instances = ctx.get_global_data(ikey) or {}
        instances[ctx.instance_id] = true
        ctx.set_global_data(ikey, instances)
    end
    M.tools = {
        { name = "judgment_passed", description = "pass", scope = "attached", parameters = {},
          handler = function(ctx) record(ctx, "passed", nil) end },
        { name = "judgment_failed", description = "fail", scope = "attached",
          parameters = { { name = "message", type = "string", description = "why" } },
          handler = function(ctx, args) record(ctx, "failed", tostring(args.message)) end },
    }
    return M
"#;

/// Drives `n_pass` passing verdicts and `n_fail` failing verdicts through a
/// fresh panel, then asserts the emitted result.
async fn run_majority_panel(n_pass: usize, n_fail: usize, fail_msg: &str) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(dir.path(), "attachable", "panel", MAJORITY_PANEL_LUA);
    let sys = build_system(dir.path());
    let origin = SessionId::from("origin".to_owned());
    let total = n_pass + n_fail;
    let mut regs = Vec::new();
    for _i in 0..total {
        let inst = PluginInstanceId::new();
        let reg = sys
            .async_handle
            .create_session_registry(vec![(inst.clone(), "panel".to_owned())], SessionId::new())
            .await
            .expect("create registry");
        sys.async_handle
            .fire_async_for_session(
                Some(reg.registry_id),
                "on_attach",
                &AttachCtx {
                    session_id: origin.to_string(),
                    plugin_name: "panel".to_owned(),
                },
                vec![inst.clone()],
            )
            .await
            .expect("fire on_attach");
        regs.push(inst);
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Post the verdicts.
    for i in 0..total {
        let child = format!("child-{i}");
        let (tool, args) = if i < n_pass {
            ("judgment_passed", json!({}))
        } else {
            ("judgment_failed", json!({ "message": fail_msg }))
        };
        sys.async_handle
            .execute_tool(
                None,
                SessionId::from(child),
                Some(origin.clone()),
                "panel",
                tool,
                &args,
            )
            .await
            .expect("execute verdict");
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let cmds = sys.captured.lock();
    cmds.iter()
        .filter(|c| c.name == "enqueue_user_message" || c.name == "push_chat_entry")
        .map(|c| c.name.clone())
        .collect()
}

#[tokio::test]
async fn majority_two_pass_one_fail_emits_pass() {
    // Given a 3-judge panel: 2 pass, 1 fail.
    // When all verdicts are posted.
    let results = run_majority_panel(2, 1, "off-topic").await;

    // Then exactly one pass entry is emitted (strict majority pass wins).
    assert_eq!(results, vec!["push_chat_entry".to_owned()]);
}

#[tokio::test]
async fn majority_one_pass_two_fail_emits_fail() {
    // Given a 3-judge panel: 1 pass, 2 fail.
    // When all verdicts are posted.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind(dir.path(), "attachable", "panel", MAJORITY_PANEL_LUA);
    let sys = build_system(dir.path());
    let origin = SessionId::from("origin".to_owned());
    let total = 3;
    for _i in 0..total {
        let inst = PluginInstanceId::new();
        let reg = sys
            .async_handle
            .create_session_registry(vec![(inst.clone(), "panel".to_owned())], SessionId::new())
            .await
            .expect("create registry");
        sys.async_handle
            .fire_async_for_session(
                Some(reg.registry_id),
                "on_attach",
                &AttachCtx {
                    session_id: origin.to_string(),
                    plugin_name: "panel".to_owned(),
                },
                vec![inst.clone()],
            )
            .await
            .expect("fire on_attach");
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Post: child-0 passes, child-1 and child-2 fail.
    let verdicts = [
        ("judgment_passed", json!({}), ""),
        (
            "judgment_failed",
            json!({ "message": "too short" }),
            "too short",
        ),
        (
            "judgment_failed",
            json!({ "message": "off-topic" }),
            "off-topic",
        ),
    ];
    for (i, (tool, args, _)) in verdicts.iter().enumerate() {
        let child = format!("child-{i}");
        sys.async_handle
            .execute_tool(
                None,
                SessionId::from(child),
                Some(origin.clone()),
                "panel",
                tool,
                args,
            )
            .await
            .expect("execute verdict");
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Then exactly one fail message is emitted, concatenating ONLY the two
    // failed reasons (order-independent: Lua table iteration is not ordered).
    let cmds = sys.captured.lock();
    let fails: Vec<String> = cmds
        .iter()
        .filter(|c| c.name == "enqueue_user_message")
        .map(|c| c.data["text"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(fails.len(), 1, "exactly one merged fail message: {fails:?}");
    let text = &fails[0];
    assert!(text.contains("too short"), "first reason present: {text}");
    assert!(text.contains("off-topic"), "second reason present: {text}");
    assert!(
        text.matches(';').count() == 1,
        "exactly one separator between two reasons: {text}"
    );
}

#[tokio::test]
async fn majority_tie_counts_as_fail() {
    // Given a 2-judge panel: 1 pass, 1 fail (tie).
    // When all verdicts are posted.
    let results = run_majority_panel(1, 1, "weak").await;

    // Then a fail message is emitted (tie → fail), not a pass entry.
    assert_eq!(results, vec!["enqueue_user_message".to_owned()]);
}

#[tokio::test]
async fn majority_single_pass_is_backward_compat_pass() {
    // Given a single judge (count=1) that passes.
    // When its verdict is posted.
    let results = run_majority_panel(1, 0, "").await;

    // Then a pass entry is emitted (1 vs 0 is a strict majority).
    assert_eq!(results, vec!["push_chat_entry".to_owned()]);
}

#[tokio::test]
async fn majority_single_fail_is_backward_compat_fail() {
    // Given a single judge (count=1) that fails.
    // When its verdict is posted.
    let results = run_majority_panel(0, 1, "bad").await;

    // Then a fail message is emitted.
    assert_eq!(results, vec!["enqueue_user_message".to_owned()]);
}
