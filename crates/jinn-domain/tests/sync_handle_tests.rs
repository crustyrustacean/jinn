//! PluginSyncHandle tests — blocking sync hook calls from actor threads.

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

use jinn_domain::SessionId;
use jinn_domain::feat::plugin_system::SessionPluginRegistry;
use jinn_domain::feat::plugin_system::{
    PluginCommand, PluginInstanceId, PluginSyncHandle, PluginSystem, PluginSystemBuildResult,
};
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
    let PluginSystemBuildResult { sync_handle, .. } = PluginSystem::build(
        dir,
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().push(cmd);
        }),
        Arc::new(|_, _, _| Box::pin(async { serde_json::Value::Null })),
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

// ── for_session tests (require both handles) ─────────────────────��──────

fn build_both(
    dir: &Path,
) -> (
    jinn_domain::feat::plugin_system::AsyncPluginHandle,
    PluginSyncHandle,
    Arc<Mutex<Vec<PluginCommand>>>,
) {
    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));
    let PluginSystemBuildResult {
        async_handle,
        sync_handle,
        ..
    } = PluginSystem::build(
        dir,
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().push(cmd);
        }),
        Arc::new(|_, _, _| Box::pin(async { serde_json::Value::Null })),
    );
    (async_handle, sync_handle, captured)
}

/// Sync `call_hooks_for_session` requires an async create_session_registry call
/// to obtain the registry ID. We run that in a block_on inside the test.
#[test]
fn sync_call_hooks_for_session_includes_global_and_session_plugins() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind_inner(
        dir.path(),
        "global",
        "g",
        r#"
        return {
            on_validate = function(ctx) return "from-global" end,
        }
    "#,
    );
    write_plugin_kind_inner(
        dir.path(),
        "attachable",
        "a",
        r#"
        return {
            on_validate = function(ctx) return "from-session" end,
        }
    "#,
    );

    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("rt")));
    let (async_handle, sync_handle, _captured) = build_both(dir.path());

    // Create a session registry that includes the attachable plugin.
    let session_id = rt
        .block_on(async {
            async_handle
                .create_session_registry(vec![(PluginInstanceId::new(), "a".to_owned())], SessionId::new())
                .await
                .expect("create registry")
        })
        .registry_id;

    let results: Vec<String> = sync_handle
        .call_hooks_for_session(
            Some(session_id),
            "on_validate",
            &ValidateCtx {
                text: String::new(),
                session_id: "s1".to_owned(),
            },
        )
        .expect("call_hooks_for_session");

    assert!(
        results.contains(&"from-global".to_owned()),
        "global: {results:?}"
    );
    assert!(
        results.contains(&"from-session".to_owned()),
        "session: {results:?}"
    );
}

#[test]
fn sync_call_hooks_for_session_excludes_unattached() {
    // Attachable plugin exists on disk but not registered. Sync call must not
    // fire it.
    let dir = tempfile::tempdir().expect("tempdir");
    write_plugin_kind_inner(
        dir.path(),
        "attachable",
        "a",
        r#"
        return {
            on_validate = function(ctx) return "leaked" end,
        }
    "#,
    );

    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("rt")));
    let (async_handle, sync_handle, _captured) = build_both(dir.path());

    // Empty registry — no attached plugins.
    let session_id = rt
        .block_on(async {
            async_handle
                .create_session_registry(vec![], SessionId::new())
                .await
                .expect("create registry")
        })
        .registry_id;

    let results: Vec<String> = sync_handle
        .call_hooks_for_session(
            Some(session_id),
            "on_validate",
            &ValidateCtx {
                text: String::new(),
                session_id: "s1".to_owned(),
            },
        )
        .expect("call_hooks_for_session");

    assert!(
        results.is_empty(),
        "attachable plugin leaked into empty session"
    );
}

fn write_plugin_kind_inner(dir: &Path, kind: &str, name: &str, lua_source: &str) {
    let plugin_dir = dir.join(kind).join(name);
    std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    std::fs::write(plugin_dir.join("init.lua"), lua_source).expect("write init.lua");
}
