//! Tests for fire_async_collect.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_in_result,
    reason = "test code"
)]

use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;

use jinn_plugin::{PluginCommand, PluginSystemBuildResult};
use serde::Serialize;

fn write_plugin(dir: &Path, name: &str, lua_source: &str) {
    let plugin_dir = dir.join(name);
    std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    std::fs::write(plugin_dir.join("init.lua"), lua_source).expect("write init.lua");
}

fn build_system(
    dir: &Path,
) -> (
    jinn_plugin::SyncPlugins,
    jinn_plugin::AsyncPluginHandle,
    Arc<Mutex<Vec<PluginCommand>>>,
) {
    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));
    let PluginSystemBuildResult {
        sync, async_handle, ..
    } = jinn_plugin::PluginSystem::build(
        dir,
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().push(cmd);
        }),
        Arc::new(|_, _, _| Box::pin(async { serde_json::Value::Null })),
    );

    (sync, async_handle, captured)
}

#[derive(Debug, Serialize)]
struct EmptyCtx {
    session_id: String,
}

#[tokio::test]
async fn collect_returns_values_from_multiple_plugins() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "alpha",
        r#"
            return {
                on_validate = function(ctx)
                    return "alpha says ok"
                end,
            }
        "#,
    );
    write_plugin(
        dir.path(),
        "beta",
        r#"
            return {
                on_validate = function(ctx)
                    return "beta says ok"
                end,
            }
        "#,
    );

    let (_, async_handle, _) = build_system(dir.path());

    let results: Vec<String> = async_handle
        .fire_async_collect(
            "on_validate",
            &EmptyCtx {
                session_id: "s1".to_owned(),
            },
        )
        .await
        .expect("fire_async_collect");

    assert_eq!(results.len(), 2);
    assert!(results.contains(&"alpha says ok".to_owned()));
    assert!(results.contains(&"beta says ok".to_owned()));
}

#[tokio::test]
async fn collect_excludes_nil_returns() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "returns_value",
        r#"
            return {
                on_check = function(ctx) return "present" end,
            }
        "#,
    );
    write_plugin(
        dir.path(),
        "returns_nil",
        "
            return {
                on_check = function(ctx) return nil end,
            }
        ",
    );

    let (_, async_handle, _) = build_system(dir.path());

    let results: Vec<String> = async_handle
        .fire_async_collect(
            "on_check",
            &EmptyCtx {
                session_id: "s1".to_owned(),
            },
        )
        .await
        .expect("fire_async_collect");

    // nil returns are excluded by the background thread.
    assert_eq!(results.len(), 1);
    assert!(results.contains(&"present".to_owned()));
}

#[tokio::test]
async fn collect_with_no_hooks_returns_empty_vec() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin(
        dir.path(),
        "no_hook",
        "
            return {
                on_other = function(ctx) end,
            }
        ",
    );

    let (_, async_handle, _) = build_system(dir.path());

    let results: Vec<String> = async_handle
        .fire_async_collect(
            "on_nonexistent",
            &EmptyCtx {
                session_id: "s1".to_owned(),
            },
        )
        .await
        .expect("fire_async_collect");

    assert!(results.is_empty());
}
