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
//!   global/                # always-loaded plugins
//!     <name>/
//!       plugin.wasm        # compiled WASM component
//!       plugin.toml        # name, description (optional, defaults to dir name)
//!   attachable/            # per-session plugins
//!     <name>/
//!       plugin.wasm
//!       plugin.toml
//! ```
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
    /// Global vs attachable.
    pub kind: PluginKind,
}

/// Sidecar `plugin.toml` fields. Only `name`/`description` are read at
/// discovery; the rest of the manifest comes from the component's
/// `get-manifest()` export at instantiation.
#[derive(Debug, Default, serde::Deserialize)]
struct PluginToml {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Discover all plugins across user + system dirs.
///
/// User plugins override system within kind. Results sorted alphabetically.
#[must_use]
pub fn discover_plugins(user_dir: &Path, system_dir: &Path) -> Vec<PluginMeta> {
    let mut seen: HashMap<(String, PluginKind), PluginMeta> = HashMap::new();

    // System plugins first (lower priority). Try new layout, fall back to flat.
    for meta in scan_kind_dir(&system_dir.join("global"), PluginKind::Global) {
        seen.entry((meta.name.clone(), meta.kind)).or_insert(meta);
    }
    for meta in scan_kind_dir(&system_dir.join("attachable"), PluginKind::Attachable) {
        seen.entry((meta.name.clone(), meta.kind)).or_insert(meta);
    }
    if !system_dir.join("global").is_dir() && !system_dir.join("attachable").is_dir() {
        for meta in scan_kind_dir(system_dir, PluginKind::Global) {
            seen.entry((meta.name.clone(), meta.kind)).or_insert(meta);
        }
    }

    // User plugins override system within kind.
    for meta in scan_kind_dir(&user_dir.join("global"), PluginKind::Global) {
        seen.insert((meta.name.clone(), meta.kind), meta);
    }
    for meta in scan_kind_dir(&user_dir.join("attachable"), PluginKind::Attachable) {
        seen.insert((meta.name.clone(), meta.kind), meta);
    }
    if !user_dir.join("global").is_dir() && !user_dir.join("attachable").is_dir() {
        for meta in scan_kind_dir(user_dir, PluginKind::Global) {
            seen.insert((meta.name.clone(), meta.kind), meta);
        }
    }

    let mut plugins: Vec<PluginMeta> = seen.into_values().collect();
    plugins.sort_by_key(|p| p.name.to_lowercase());
    plugins
}

/// Scan one kind directory for plugin subdirectories.
fn scan_kind_dir(dir: &Path, kind: PluginKind) -> Vec<PluginMeta> {
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
        let (resolved_name, description) = parse_sidecar(&sidecar, &name);

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
fn parse_sidecar(sidecar: &Path, dir_name: &str) -> (String, Option<String>) {
    let toml_str = match std::fs::read_to_string(sidecar) {
        Ok(s) => s,
        Err(_) => return (dir_name.to_owned(), None),
    };
    let parsed: PluginToml = toml::from_str(&toml_str).unwrap_or_default();
    (
        parsed.name.unwrap_or_else(|| dir_name.to_owned()),
        parsed.description,
    )
}
