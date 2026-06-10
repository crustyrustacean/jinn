//! Plugin data tests — the critical cross-context tests.
//!
//! These tests verify that:
//! 1. Async hooks can write to plugin_data and sync hooks can read it
//! 2. Plugin data is absent (nil) when no async hook has written yet
//! 3. Multiple plugins have isolated data

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_in_result,
    reason = "test code"
)]

use std::path::Path;
use std::sync::{Arc, Mutex};

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
    #[expect(dead_code, reason = "captured commands unused in these tests")]
    captured: Arc<Mutex<Vec<PluginCommand>>>,
    sync: jinn_plugin::SyncPlugins,
    async_handle: jinn_plugin::AsyncPluginHandle,
}

fn build_system(dir: &Path) -> TestSystem {
    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));
    let PluginSystemBuildResult { sync, async_handle, .. } = PluginSystem::build(
        dir,
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().expect("lock").push(cmd);
        }),
        Arc::new(|name, _data| {
            let name = name.to_owned();
            Box::pin(async move {
                if name == "llm" {
                    json!("FAIL")
                } else {
                    json!(null)
                }
            })
        }),
    );

    TestSystem {
        captured,
        sync,
        async_handle,
    }
}

#[derive(Debug, Serialize)]
struct FilterCtx {
    text: String,
    session_id: String,
}

#[derive(Debug, Serialize)]
struct TurnEndCtx {
    session_id: String,
    last_assistant_message: String,
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn async_write_sync_read_plugin_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "auto_judge",
        r#"
            local M = {}

            function M.on_turn_end(ctx)
                local verdict = ctx.request("llm", { prompt = "evaluate" })
                ctx.set_plugin_data({
                    verdict = verdict,
                    timestamp = 12345,
                })
            end

            function M.on_filter_input(ctx)
                local data = ctx.plugin_data
                if data and data.verdict == "FAIL" then
                    return "⚠ " .. ctx.text
                end
                return ctx.text
            end

            return M
        "#,
    );

    let system = build_system(dir.path());

    // Step 1: fire async hook (writes plugin_data).
    system
        .async_handle
        .fire_async(
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: "bad response".to_owned(),
            },
        )
        .await
        .expect("async fire");

    // Step 2: call sync hook (reads plugin_data).
    let result: String = system
        .sync
        .sync_hooks("on_filter_input")
        .next()
        .expect("should have hook")
        .call(&FilterCtx {
            text: "hello".to_owned(),
            session_id: "s1".to_owned(),
        })
        .expect("sync call");

    // The async hook wrote verdict="FAIL", so sync hook prefixes with ⚠.
    assert_eq!(result, "⚠ hello");
}

#[test]
fn plugin_data_absent_returns_nil_in_ctx() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "eager",
        r#"
            local M = {}
            function M.on_filter_input(ctx)
                -- ctx.plugin_data is nil because no async hook has written yet.
                if ctx.plugin_data == nil then
                    return ctx.text .. " (no data)"
                end
                return ctx.text
            end
            return M
        "#,
    );

    let system = build_system(dir.path());

    let result: String = system
        .sync
        .sync_hooks("on_filter_input")
        .next()
        .expect("should have hook")
        .call(&FilterCtx {
            text: "hello".to_owned(),
            session_id: "s1".to_owned(),
        })
        .expect("call");

    assert_eq!(result, "hello (no data)");
}

#[tokio::test]
async fn multiple_plugins_have_isolated_plugin_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "alpha",
        r#"
            return {
                on_turn_end = function(ctx)
                    ctx.set_plugin_data({ value = "alpha_result" })
                end,
                on_check = function(ctx)
                    return ctx.plugin_data.value
                end,
            }
        "#,
    );
    write_plugin(
        dir.path(),
        "beta",
        r#"
            return {
                on_turn_end = function(ctx)
                    ctx.set_plugin_data({ value = "beta_result" })
                end,
                on_check = function(ctx)
                    return ctx.plugin_data.value
                end,
            }
        "#,
    );

    let system = build_system(dir.path());

    // Fire async for both plugins.
    system
        .async_handle
        .fire_async(
            "on_turn_end",
            &TurnEndCtx {
                session_id: "s1".to_owned(),
                last_assistant_message: String::new(),
            },
        )
        .await
        .expect("async fire");

    // Each sync hook sees only its own data.
    let results: Vec<String> = system
        .sync
        .sync_hooks("on_check")
        .map(|h| {
            h.call::<_, String>(&FilterCtx {
                text: String::new(),
                session_id: "s1".to_owned(),
            })
            .expect("call")
        })
        .collect();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&"alpha_result".to_owned()));
    assert!(results.contains(&"beta_result".to_owned()));
}
