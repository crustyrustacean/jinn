//! PluginSyncHandle tests — blocking sync hook calls from actor threads.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_in_result,
    reason = "test code"
)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use jinn_plugin::{PluginCommand, PluginSystem, PluginSyncHandle};
use serde::Serialize;

fn write_plugin(dir: &Path, name: &str, lua_source: &str) {
    let plugin_dir = dir.join(name);
    std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    std::fs::write(plugin_dir.join("init.lua"), lua_source).expect("write init.lua");
}

fn build_system(dir: &Path) -> (PluginSyncHandle, Arc<Mutex<Vec<PluginCommand>>>) {
    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));
    let (_, _, sync_handle) = PluginSystem::new(
        dir,
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().expect("lock").push(cmd);
        }),
        Arc::new(|_, _| serde_json::Value::Null),
    );

    (sync_handle, captured)
}

#[derive(Debug, Serialize)]
struct ValidateCtx {
    text: String,
    session_id: String,
}

#[test]
fn sync_handle_returns_collected_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "validator_a",
        r#"
            return {
                on_validate = function(ctx)
                    return "error_from_a: " .. ctx.text
                end,
            }
        "#,
    );
    write_plugin(
        dir.path(),
        "validator_b",
        r#"
            return {
                on_validate = function(ctx)
                    return "error_from_b: " .. ctx.text
                end,
            }
        "#,
    );

    let (sync_handle, _) = build_system(dir.path());

    let results: Vec<String> = sync_handle
        .call_hooks(
            "on_validate",
            &ValidateCtx {
                text: "test".to_owned(),
                session_id: "s1".to_owned(),
            },
        )
        .expect("call_hooks");

    assert_eq!(results.len(), 2);
    assert!(results.contains(&"error_from_a: test".to_owned()));
    assert!(results.contains(&"error_from_b: test".to_owned()));
}

#[test]
fn sync_handle_empty_when_no_hooks() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "empty",
        r#"
            return {
                on_other = function(ctx) return "hello" end
            }
        "#,
    );

    let (sync_handle, _) = build_system(dir.path());

    let results: Vec<String> = sync_handle
        .call_hooks(
            "on_validate",
            &ValidateCtx {
                text: String::new(),
                session_id: "s1".to_owned(),
            },
        )
        .expect("call_hooks");

    assert!(results.is_empty());
}

#[test]
fn sync_handle_excludes_nil() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "maybe",
        r#"
            return {
                on_validate = function(ctx)
                    if ctx.text == "skip" then return nil end
                    return "present"
                end,
            }
        "#,
    );

    let (sync_handle, _) = build_system(dir.path());

    let results: Vec<String> = sync_handle
        .call_hooks(
            "on_validate",
            &ValidateCtx {
                text: "skip".to_owned(),
                session_id: "s1".to_owned(),
            },
        )
        .expect("call_hooks");

    assert!(results.is_empty());
}
