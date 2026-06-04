//! Async hook tests.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_in_result,
    reason = "test code"
)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use jinn_plugin::{PluginCommand, PluginSystem};
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
    let (_, async_handle, _) = PluginSystem::new(
        dir,
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().expect("lock").push(cmd);
        }),
        Arc::new(|name, data| {
            // Default request handler: echo back for "llm", null otherwise.
            if name == "llm" {
                json!(format!("response_to: {}", data["prompt"]))
            } else {
                json!(null)
            }
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
    let cmds = sys.captured.lock().expect("lock");
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
    let cmds = sys.captured.lock().expect("lock");
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].name, "enqueue_user_message");
    assert_eq!(cmds[0].data["text"], "follow up");
}
