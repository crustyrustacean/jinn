//! Plugin discovery tests — `.wasm` + sidecar `plugin.toml` scanning.

use std::fs;
use std::path::Path;

use jinn_wasm_host::discovery::{PluginKind, PluginMeta, discover_plugins};

/// Create a plugin directory with an empty `.wasm` placeholder + optional sidecar.
fn make_plugin(base: &Path, kind: &str, name: &str, sidecar: Option<&str>) {
    let dir = base.join(kind).join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("plugin.wasm"), b"\0asm").unwrap();
    if let Some(toml) = sidecar {
        fs::write(dir.join("plugin.toml"), toml).unwrap();
    }
}

#[test]
fn discovers_global_plugin_without_sidecar() {
    // Given a user dir with one global plugin, no sidecar.
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    make_plugin(user.path(), "global", "alpha", None);

    // When discovering.
    let plugins = discover_plugins(user.path(), system.path());

    // Then one global plugin named "alpha" is found.
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].name, "alpha");
    assert_eq!(plugins[0].kind, PluginKind::Global);
    assert!(plugins[0].description.is_none());
}

#[test]
fn discovers_attachable_plugin() {
    // Given a user dir with one attachable plugin.
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    make_plugin(user.path(), "attachable", "beta", None);

    // When discovering.
    let plugins = discover_plugins(user.path(), system.path());

    // Then one attachable plugin named "beta" is found.
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].kind, PluginKind::Attachable);
}

#[test]
fn sidecar_overrides_name_and_description() {
    // Given a plugin with a sidecar.
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    make_plugin(
        user.path(),
        "global",
        "coded-name",
        Some("name = \"Friendly Name\"\ndescription = \"Does the thing\"\n"),
    );

    // When discovering.
    let plugins = discover_plugins(user.path(), system.path());

    // Then the sidecar name + description win.
    assert_eq!(plugins[0].name, "Friendly Name");
    assert_eq!(plugins[0].description.as_deref(), Some("Does the thing"));
}

#[test]
fn user_plugin_overrides_system_within_kind() {
    // Given the same plugin name in system + user (both global).
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    make_plugin(
        system.path(),
        "global",
        "dup",
        Some("description = \"system\"\n"),
    );
    make_plugin(
        user.path(),
        "global",
        "dup",
        Some("description = \"user\"\n"),
    );

    // When discovering.
    let plugins = discover_plugins(user.path(), system.path());

    // Then only the user version survives (override within kind).
    let globals: Vec<&PluginMeta> = plugins
        .iter()
        .filter(|p| p.kind == PluginKind::Global && p.name == "dup")
        .collect();
    assert_eq!(globals.len(), 1);
    assert_eq!(globals[0].description.as_deref(), Some("user"));
}

#[test]
fn same_name_global_and_attachable_coexist() {
    // Given a plugin named "x" as both global (system) and attachable (user).
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    make_plugin(system.path(), "global", "x", None);
    make_plugin(user.path(), "attachable", "x", None);

    // When discovering.
    let plugins = discover_plugins(user.path(), system.path());

    // Then both are kept — override is within-kind only.
    assert_eq!(plugins.len(), 2);
}

#[test]
fn skips_dotfiles() {
    // Given a hidden plugin directory.
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    make_plugin(user.path(), "global", ".hidden", None);

    // When discovering.
    let plugins = discover_plugins(user.path(), system.path());

    // Then the hidden plugin is skipped.
    assert!(plugins.is_empty());
}

#[test]
fn flat_fallback_scans_dir_as_global() {
    // Given a system dir with NO global/ or attachable/ subdirs, just plugins.
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    make_plugin(system.path(), "", "flat", None);

    // When discovering.
    let plugins = discover_plugins(user.path(), system.path());

    // Then flat-fallback finds it as global.
    assert!(
        plugins
            .iter()
            .any(|p| p.name == "flat" && p.kind == PluginKind::Global)
    );
}

#[test]
fn results_sorted_alphabetically() {
    // Given plugins in non-alphabetical creation order.
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    make_plugin(user.path(), "global", "zebra", None);
    make_plugin(user.path(), "global", "alpha", None);
    make_plugin(user.path(), "global", "mango", None);

    // When discovering.
    let plugins = discover_plugins(user.path(), system.path());

    // Then the names are sorted case-insensitively.
    let names: Vec<&str> = plugins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "mango", "zebra"]);
}

#[test]
fn ignores_dir_without_wasm_file() {
    // Given a plugin directory missing its .wasm.
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();
    let dir = user.path().join("global").join("no-wasm");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("plugin.toml"), b"name = \"x\"\n").unwrap();

    // When discovering.
    let plugins = discover_plugins(user.path(), system.path());

    // Then the directory is skipped (no .wasm found).
    assert!(plugins.is_empty());
}
