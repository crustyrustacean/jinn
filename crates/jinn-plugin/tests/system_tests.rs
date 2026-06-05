//! Integration tests for the plugin system.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_in_result,
    reason = "test code"
)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use jinn_plugin::{AsyncPluginHandle, PluginCommand, PluginSystem, SyncPlugins};
use serde::Serialize;

// ── Test Helpers ─────────────────────────────────────────────────────────

/// Build a PluginSystem from plugins in the given directory.
fn build_system(
    dir: &Path,
) -> (
    SyncPlugins,
    AsyncPluginHandle,
    jinn_plugin::PluginSyncHandle,
) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let captured: Arc<Mutex<Vec<PluginCommand>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();

    let (sync, async_handle, sync_handle) = PluginSystem::new(
        dir,
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_clone.lock().expect("lock").push(cmd);
        }),
        Arc::new(|name, _data| {
            tracing::warn!(name, "no request handler in test");
            serde_json::Value::Null
        }),
    );

    // Leak the runtime so it stays alive for the test duration.
    std::mem::forget(rt);

    (sync, async_handle, sync_handle)
}

/// Write a plugin to a temp directory.
fn write_plugin(dir: &Path, name: &str, lua_source: &str) {
    let plugin_dir = dir.join(name);
    std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    std::fs::write(plugin_dir.join("init.lua"), lua_source).expect("write init.lua");
}

/// Get captured commands from the test system.
fn captured_commands(_sync: &SyncPlugins) -> Vec<PluginCommand> {
    // We can't access the captured vector directly since it's moved into the closure.
    // This is a placeholder — tests that need to inspect commands should build
    // their own capture mechanism.
    Vec::new()
}

// ── Context structs for tests ────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct FilterCtx {
    text: String,
    session_id: String,
}

// ── Phase 1 Tests ────────────────────────────────────────────────────────

#[test]
fn plugin_system_constructs_with_empty_plugin_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (sync, _, _) = build_system(dir.path());

    // No plugins loaded → hook count is 0.
    assert_eq!(sync.plugin_count(), 0);
}

#[test]
fn plugin_system_constructs_with_nonexistent_dirs() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let (sync, _, _) = PluginSystem::new(
        Path::new("/nonexistent/user"),
        Path::new("/nonexistent/system"),
        rt.handle().clone(),
        Arc::new(|_| {}),
        Arc::new(|_, _| serde_json::Value::Null),
    );
    std::mem::forget(rt);
    assert_eq!(sync.plugin_count(), 0);
}
