//! Sync hook tests.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_in_result,
    reason = "test code"
)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use jinn_domain::feat::plugin_dispatch::HookContext;
use jinn_domain::feat::plugin_system::{
    PluginCommand, PluginSystem, PluginSystemBuildResult, SyncPlugins,
};

// ── Helpers ──────────────────────────────────────────────────────────────

fn write_plugin(dir: &Path, name: &str, lua_source: &str) {
    let plugin_dir = dir.join(name);
    std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    std::fs::write(plugin_dir.join("init.lua"), lua_source).expect("write init.lua");
}

fn build_system_with_capture(dir: &Path) -> (SyncPlugins, Arc<Mutex<Vec<PluginCommand>>>) {
    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let PluginSystemBuildResult { sync, .. } = PluginSystem::build(
        dir,
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().expect("lock").push(cmd);
        }),
        Arc::new(|_, _| Box::pin(async { serde_json::Value::Null })),
    );
    std::mem::forget(rt);
    (sync, captured)
}

fn write_attachable_plugin(dir: &Path, name: &str, lua_source: &str) {
    let attachable_dir = dir.join("attachable");
    write_plugin(&attachable_dir, name, lua_source);
}

// ── Tests ────────────────────────────────────────────────────────────────

#[test]
fn sync_hook_returns_deserialized_value() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "censor",
        r#"
            local M = {}
            function M.on_filter_input(ctx)
                return ctx.text:gsub("bad", "***")
            end
            return M
        "#,
    );

    let (sync, _) = build_system_with_capture(dir.path());

    let results: Vec<String> = sync
        .sync_hooks("on_filter_input")
        .map(|h| {
            h.call::<String>(&HookContext::from(serde_json::json!({
                "text": "this is bad",
                "session_id": "s1",
            })))
            .expect("hook call")
        })
        .collect();

    assert_eq!(results, vec!["this is ***"]);
}

#[test]
fn sync_hooks_skips_plugins_without_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "has_hook",
        "return { on_filter_input = function(ctx) return ctx.text end }",
    );
    write_plugin(
        dir.path(),
        "no_hook",
        "return { on_other = function(ctx) end }",
    );

    let (sync, _) = build_system_with_capture(dir.path());

    let hooks: Vec<_> = sync.sync_hooks("on_filter_input").collect();
    assert_eq!(hooks.len(), 1);
}

#[test]
fn sync_hook_returns_nil_deserializes_to_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "maybe",
        "
            return { on_validate = function(ctx) return nil end }
        ",
    );

    let (sync, _) = build_system_with_capture(dir.path());

    let result: Option<String> = sync
        .sync_hooks("on_validate")
        .next()
        .expect("should have hook")
        .call(&HookContext::from(serde_json::json!({
            "text": "test",
            "session_id": "s1",
        })))
        .expect("call");

    assert_eq!(result, None);
}

#[test]
fn sync_hook_script_error_returns_err() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "broken",
        r#"
            return {
                on_filter_input = function(ctx)
                    error("intentional error")
                end,
            }
        "#,
    );

    let (sync, _) = build_system_with_capture(dir.path());

    let result: Result<String, _> = sync
        .sync_hooks("on_filter_input")
        .next()
        .expect("should have hook")
        .call(&HookContext::from(serde_json::json!({
            "text": "hello",
            "session_id": "s1",
        })));

    assert!(result.is_err(), "script error should be Err not panic");
}

#[test]
fn sync_hook_emit_sends_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "emitter",
        r#"
            local M = {}
            function M.on_turn_end(ctx)
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    message = "hello from plugin",
                })
            end
            return M
        "#,
    );

    let (sync, captured) = build_system_with_capture(dir.path());

    // Fire the hook — it will call ctx.emit which goes through the channel.
    // The drainer picks it up asynchronously.
    let _: () = sync
        .sync_hooks("on_turn_end")
        .next()
        .expect("should have hook")
        .call(&HookContext::from(serde_json::json!({
            "text": "",
            "session_id": "s1",
        })))
        .expect("call");

    // Give the drainer task time to process.
    std::thread::sleep(std::time::Duration::from_millis(100));

    let cmds = captured.lock().expect("lock");
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].name, "push_chat_entry");
    assert_eq!(cmds[0].data["session_id"], "s1");
    assert_eq!(cmds[0].data["message"], "hello from plugin");
}

#[test]
fn attachable_plugin_sync_hook_is_loaded_and_callable() {
    // Given an attachable plugin that defines on_session_preview.
    let dir = tempfile::tempdir().expect("tempdir");
    write_attachable_plugin(
        dir.path(),
        "test_judge",
        r#"
            local M = {}
            function M.on_session_preview(ctx)
                return { session_id = "judge-123" }
            end
            return M
        "#,
    );

    let (sync, _) = build_system_with_capture(dir.path());

    // When calling the on_session_preview hook.
    let results: Vec<serde_json::Value> = sync
        .sync_hooks("on_session_preview")
        .map(|h| {
            h.call::<serde_json::Value>(&HookContext::from(serde_json::json!({
                "text": "",
                "session_id": "origin-session",
            })))
            .expect("hook call")
        })
        .collect();

    // Then the attachable plugin's hook responds with the expected value.
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["session_id"], "judge-123");
}
