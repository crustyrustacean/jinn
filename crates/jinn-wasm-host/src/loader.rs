//! Component loading — compile discovered `.wasm` files and instantiate them.
//!
//! This is the orchestration layer that ties together the engine, the
//! dual-store model, and the bag layer. It does NOT fire hooks (that's
//! `hooks.rs`) and does NOT implement the host imports (that's `imports.rs`).
//! It compiles each discovered plugin once, instantiates it in both the async
//! and sync store sets under a given identity, and reads its `manifest()` to
//! cache declared keybinds + tools.
//!
//! # Why cache the manifest?
//!
//! The `manifest()` export returns the plugin's keybinds + tool declarations.
//! The host needs these BEFORE any hook fires (the keymap registers keybinds
//! at startup; tool discovery happens before any session). Reading them once
//! at load time and caching avoids re-entering the component for metadata.

use std::collections::HashMap;
use std::path::Path;

use wasmtime::component::Linker;

use crate::bag::{GlobalBagStore, InstanceBagStore};
use crate::discovery::{PluginKind, PluginMeta, discover_plugins};
use crate::engine::{CompiledComponent, EngineConfig, WasmEngine};
use crate::store::{InstanceCtx, StoreKind, StoreSet};

/// Cached manifest metadata for a loaded plugin — declared keybinds + tools.
///
/// Populated once at load time by calling the component's `get-manifest()`
/// export. The wiring layer (Phase 3) reads this to register keybinds into the
/// keymap and tools into the session's tool registry.
#[derive(Debug, Clone, Default)]
pub struct CachedManifest {
    /// Declared keybinds (scope, keys, action, description).
    pub keybinds: Vec<ManifestKeybind>,
    /// Declared plugin-defined tools.
    pub tools: Vec<ManifestTool>,
    /// Human-readable description (sidecar + manifest merge).
    pub description: Option<String>,
}

/// One keybind declared by a plugin's manifest.
#[derive(Debug, Clone)]
pub struct ManifestKeybind {
    pub plugin_name: String,
    pub scope: String,
    pub keys: String,
    /// The async hook name to fire when pressed (e.g. `"on-enrich"`).
    pub action: String,
    pub description: String,
}

/// One tool declared by a plugin's manifest.
#[derive(Debug, Clone)]
pub struct ManifestTool {
    pub name: String,
    pub description: String,
    pub global: bool,
}

/// Convert a WIT `Manifest` into the host-side `CachedManifest`.
///
/// The WIT record is produced by the plugin's `get-manifest()` export;
/// this flattens it into the keybind/tool metadata the wiring layer reads.
pub(crate) fn convert_manifest(
    plugin_name: &str,
    manifest: crate::bindings::exports::jinn::plugin::hooks::Manifest,
) -> CachedManifest {
    use crate::bindings::jinn::plugin::types::{Keybind, ToolDecl, ToolScope};

    let keybinds = manifest
        .keybinds
        .into_iter()
        .map(|kb: Keybind| ManifestKeybind {
            plugin_name: plugin_name.to_owned(),
            scope: kb.scope,
            keys: kb.keys,
            action: kb.action,
            description: kb.description,
        })
        .collect();

    let tools = manifest
        .tools
        .into_iter()
        .map(|t: ToolDecl| ManifestTool {
            name: t.name,
            description: t.description,
            global: matches!(t.scope, ToolScope::Global),
        })
        .collect();

    CachedManifest {
        keybinds,
        tools,
        description: manifest.description,
    }
}

/// A plugin compiled and ready to instantiate, plus its discovery metadata.
#[derive(Debug, Clone)]
pub struct CompiledPlugin {
    pub meta: PluginMeta,
    pub component: CompiledComponent,
}

/// Error loading plugins.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct PluginLoadError;

/// Compile every discovered plugin against the shared engine.
///
/// Reads each `.wasm` from disk, compiles it once (the resulting
/// `CompiledComponent` is `Clone` and cheap to share via `Arc`). Discovery
/// metadata is preserved alongside the compiled bytes so the wiring layer knows
/// each plugin's name, kind, and description without re-reading the sidecar.
///
/// # Errors
///
/// Returns an error if any `.wasm` fails to read or compile.
pub fn compile_discovered(
    engine: &WasmEngine,
    user_dir: &Path,
    system_dir: &Path,
) -> Result<Vec<CompiledPlugin>, error_stack::Report<PluginLoadError>> {
    let metas = discover_plugins(user_dir, system_dir);
    compile_metas(engine, &metas)
}

fn compile_metas(
    engine: &WasmEngine,
    metas: &[PluginMeta],
) -> Result<Vec<CompiledPlugin>, error_stack::Report<PluginLoadError>> {
    let mut compiled = Vec::with_capacity(metas.len());
    for meta in metas {
        let bytes = std::fs::read(&meta.path).map_err(|e| {
            error_stack::Report::new(PluginLoadError)
                .attach(e.to_string())
                .attach(meta.path.to_string_lossy().to_string())
                .attach("reading plugin .wasm")
        })?;
        let component = CompiledComponent::compile(engine, &bytes).map_err(|e| {
            error_stack::Report::new(PluginLoadError)
                .attach(e.to_string())
                .attach(meta.name.clone())
                .attach("compiling plugin .wasm")
        })?;
        compiled.push(CompiledPlugin {
            meta: meta.clone(),
            component,
        });
    }
    Ok(compiled)
}

/// The per-thread instantiation result: one populated store set.
///
/// Built once per thread (async host thread, render thread) from the same
/// compiled plugins. Holds the `!Send` `StoreSet` and the cached manifests
/// (manifests are read from whichever store finishes first; they're identical
/// across stores since the same component bytes produce the same manifest).
impl std::fmt::Debug for LoadedStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedStore")
            .field("storeset", &self.storeset)
            .field("manifests", &self.manifests)
            .finish()
    }
}

pub struct LoadedStore {
    /// The `!Send` store set. Lives on exactly one thread.
    pub storeset: StoreSet,
    /// Manifests cached at load time, keyed by plugin name.
    pub manifests: HashMap<String, CachedManifest>,
}

/// Build the bag layer shared by both store sets.
///
/// Both the async and sync store sets reference the SAME bag layer (cloned by
/// `Arc`), so a write on one thread is visible to the other on its next read —
/// mirroring the old Lua system's two Lua states sharing one `PluginData`.
#[must_use]
pub fn shared_bag_layer() -> (InstanceBagStore, GlobalBagStore) {
    (InstanceBagStore::new(), GlobalBagStore::new())
}

/// Instantiate the global plugins from a slice into an async store set.
///
pub async fn load_globals_into_async(
    store: &mut LoadedStore,
    plugins: &[CompiledPlugin],
    linker: &Linker<crate::store::StoreState>,
) -> Result<(), error_stack::Report<PluginLoadError>> {
    for plugin in plugins {
        if plugin.meta.kind != PluginKind::Global {
            continue;
        }
        load_one_async(store, plugin, linker).await?;
    }
    Ok(())
}

/// Instantiate one plugin into a store set under a given identity (async).
async fn load_one_async(
    store: &mut LoadedStore,
    plugin: &CompiledPlugin,
    linker: &Linker<crate::store::StoreState>,
) -> Result<(), error_stack::Report<PluginLoadError>> {
    let ctx = InstanceCtx {
        plugin_name: plugin.meta.name.clone(),
        instance_id: synthetic_global_id(&plugin.meta.name),
        session_id: None,
    };
    let manifest = store
        .storeset
        .load_async(&plugin.component, ctx, linker)
        .await
        .map_err(|e| error_stack::Report::new(PluginLoadError).attach(e.to_string()))?;
    store.manifests.insert(
        plugin.meta.name.clone(),
        convert_manifest(&plugin.meta.name, manifest),
    );
    Ok(())
}

/// Synthesize a deterministic-ish instance id for a global plugin.
///
/// Global plugins have no session to derive identity from, so we mint a
/// per-plugin-name id. It must be stable across the two store sets (sync +
/// async) so both instances map to the same bag slot.
pub(crate) fn synthetic_global_id(name: &str) -> jinn_core_types::PluginInstanceId {
    // Use a hash-derived string so both the sync and async store sets key
    // the same bag slot for a given global plugin name. The id itself is
    // opaque to the host; it just needs to be stable + unique-per-name.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    format!("i-global-{:016x}", hasher.finish()).into()
}

/// Construct an empty `LoadedStore` for a given thread kind.
///
/// The wiring layer calls this once per thread (async host thread, render
/// thread), then `load_globals` to populate it.
#[must_use]
pub fn new_loaded_store(
    kind: StoreKind,
    engine: &WasmEngine,
    bags: &InstanceBagStore,
    globals: &GlobalBagStore,
) -> LoadedStore {
    LoadedStore {
        storeset: StoreSet::new(kind, engine.inner().clone(), bags.clone(), globals.clone()),
        manifests: HashMap::new(),
    }
}

/// Construct the shared engine used by all stores.
///
/// # Errors
///
/// Returns an error if the wasmtime engine cannot be configured.
pub fn build_engine(
    config: &EngineConfig,
) -> Result<WasmEngine, error_stack::Report<crate::engine::EngineConfigError>> {
    WasmEngine::new(config)
}
