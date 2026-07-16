//! `.wasm` + sidecar `plugin.toml` plugin discovery.
//!
//! Mirrors the old Lua `discover_plugins` layout and override rules, but
//! scans for `.wasm` files accompanied by a `plugin.toml` sidecar (WASM has
//! no header comments, so metadata lives in the sidecar).
//!
//! # Layout
//!
//! ```text
//! <plugins-dir>/
//!   global/                # where global plugins are placed by the build
//!     <name>/
//!       plugin.wasm        # compiled WASM component
//!       plugin.toml        # name, description, kind (optional, defaults to dir)
//!   attachable/            # where attachable plugins are placed by the build
//!     <name>/
//!       plugin.wasm
//!       plugin.toml
//! ```
//!
//! # Kind source of truth
//!
//! The plugin's `kind` (global vs attachable) is declared in its `plugin.toml`
//! as `kind = "global"` or `kind = "attachable"`. This is the authoritative
//! source — the directory it happens to live under (`global/` vs `attachable/`)
//! is only the build's default placement and is ignored for classification.
//! When `kind` is absent from the sidecar, the directory it was discovered in
//! is used as a fallback.
//!
//! # Override rule
//!
//! User plugins override system plugins by name *within the same kind*.
//! Global and attachable namespaces are independent. Flat-fallback: if neither
//! `global/` nor `attachable/` exists, the dir is scanned directly as global.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Whether a plugin is loaded once at startup or attached per-session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginKind {
    /// Loaded once at startup; hooks fire for every session.
    Global,
    /// Attached to individual sessions on demand.
    Attachable,
}

/// Discovered plugin metadata.
#[derive(Debug, Clone)]
pub struct PluginMeta {
    /// Plugin name (directory name, or sidecar override).
    pub name: String,
    /// Path to the `.wasm` file.
    pub path: PathBuf,
    /// Human-readable description from the sidecar `plugin.toml`.
    pub description: Option<String>,
    /// Global vs attachable — declared in the sidecar, defaulted from the dir.
    pub kind: PluginKind,
}

/// Sidecar `plugin.toml` fields. Only `name`/`description`/`kind` are read at
/// discovery; the rest of the manifest comes from the component's
/// `get-manifest()` export at instantiation.
///
/// `kind` is the authoritative plugin kind. When absent, discovery falls back
/// to the directory the plugin was found under (`global/` → Global).
#[derive(Debug, Default, serde::Deserialize)]
struct PluginToml {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    kind: Option<PluginKindToml>,
}

/// TOML representation of [`PluginKind`] (`"global"` / `"attachable"`).
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum PluginKindToml {
    Global,
    Attachable,
}

impl From<PluginKindToml> for PluginKind {
    fn from(k: PluginKindToml) -> Self {
        match k {
            PluginKindToml::Global => PluginKind::Global,
            PluginKindToml::Attachable => PluginKind::Attachable,
        }
    }
}

/// Discover all plugins across user + system dirs.
///
/// User plugins override system within kind. Results sorted alphabetically.
#[must_use]
pub fn discover_plugins(user_dir: &Path, system_dir: &Path) -> Vec<PluginMeta> {
    let mut seen: HashMap<(String, PluginKind), PluginMeta> = HashMap::new();

    // System plugins first (lower priority). Try new layout, fall back to flat.
    for meta in scan_kind_dir(&system_dir.join("global"), PluginKind::Global) {
        seen.entry(meta_key(&meta)).or_insert(meta);
    }
    for meta in scan_kind_dir(&system_dir.join("attachable"), PluginKind::Attachable) {
        seen.entry(meta_key(&meta)).or_insert(meta);
    }
    if !system_dir.join("global").is_dir() && !system_dir.join("attachable").is_dir() {
        for meta in scan_kind_dir(system_dir, PluginKind::Global) {
            seen.entry(meta_key(&meta)).or_insert(meta);
        }
    }

    // User plugins override system within kind.
    for meta in scan_kind_dir(&user_dir.join("global"), PluginKind::Global) {
        seen.insert(meta_key(&meta), meta);
    }
    for meta in scan_kind_dir(&user_dir.join("attachable"), PluginKind::Attachable) {
        seen.insert(meta_key(&meta), meta);
    }
    if !user_dir.join("global").is_dir() && !user_dir.join("attachable").is_dir() {
        for meta in scan_kind_dir(user_dir, PluginKind::Global) {
            seen.insert(meta_key(&meta), meta);
        }
    }

    let mut plugins: Vec<PluginMeta> = seen.into_values().collect();
    plugins.sort_by_key(|p| p.name.to_lowercase());
    plugins
}

/// Dedup key for a plugin: (name, kind).
fn meta_key(meta: &PluginMeta) -> (String, PluginKind) {
    (meta.name.clone(), meta.kind)
}

/// Scan one kind directory for plugin subdirectories.
///
/// `dir_kind` is the fallback kind when the sidecar doesn't declare one.
fn scan_kind_dir(dir: &Path, dir_kind: PluginKind) -> Vec<PluginMeta> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Find the .wasm component inside the dir.
        let Some(wasm_path) = find_wasm(&path) else {
            continue;
        };

        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_owned(),
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }

        let sidecar = path.join("plugin.toml");
        let (resolved_name, description, kind) = parse_sidecar(&sidecar, &name, dir_kind);

        found.push(PluginMeta {
            name: resolved_name,
            path: wasm_path,
            description,
            kind,
        });
    }
    found
}

/// Find the first `*.wasm` file in a plugin directory.
fn find_wasm(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "wasm") && p.is_file())
}

/// Read the sidecar `plugin.toml`. Defaults name to the directory name.
///
/// Returns `(name, description, kind)`. `kind` comes from the sidecar's `kind`
/// field when present; otherwise falls back to `dir_kind` (the directory the
/// plugin was discovered under).
fn parse_sidecar(
    sidecar: &Path,
    dir_name: &str,
    dir_kind: PluginKind,
) -> (String, Option<String>, PluginKind) {
    let toml_str = match std::fs::read_to_string(sidecar) {
        Ok(s) => s,
        Err(_) => return (dir_name.to_owned(), None, dir_kind),
    };
    let parsed: PluginToml = toml::from_str(&toml_str).unwrap_or_default();
    let kind = parsed.kind.map(PluginKind::from).unwrap_or(dir_kind);
    (
        parsed.name.unwrap_or_else(|| dir_name.to_owned()),
        parsed.description,
        kind,
    )
}
