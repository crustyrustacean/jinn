//! Render-thread sync plugins — direct hook calls on the sync store set.
//!
//! [`SyncWasmPlugins`] is `!Send` because `wasmtime::Store` is `!Send`. It lives
//! on the render thread and calls sync hook exports (`on-chat-input-badges-render`,
//! `on-keybind-trigger`, `on-submit-intercept`, `on-session-preview`) directly,
//! with zero channel hops. This mirrors the old `jinn_plugin::SyncPlugins` which
//! owned a `!Send` Lua state.
//!
//! The trait keeps the old `serde_json::Value` shape (`PluginSyncHooks`);
//! internally each call is routed through [`dispatch::dispatch_sync_hook`], which
//! builds the typed WIT ctx record from the JSON, calls the typed `call_*` method,
//! and converts the typed result back to JSON.

use std::cell::RefCell;

use serde_json::Value;

use jinn_domain::feat::plugin_dispatch::HookContext;

use crate::store::{StoreKind, StoreSet};

/// Error raised by a sync render-thread hook failure. Colocated with
/// [`SyncWasmPlugins`] — the sole producer.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct SyncHookError;

/// Render-thread plugin store. `!Send`.
pub struct SyncWasmPlugins {
    // RefCell: the trait method takes &self but each hook call mutably
    // borrows its Store. Single-threaded (render thread only), so this is safe.
    store: RefCell<StoreSet>,
    // Keybinds declared by loaded plugins' manifests. Populated when
    // instances are loaded; read by the keymap during launch.
    keybinds: RefCell<Vec<crate::loader::ManifestKeybind>>,
}

impl std::fmt::Debug for SyncWasmPlugins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncWasmPlugins")
            .field("instances", &self.store.borrow().len())
            .finish_non_exhaustive()
    }
}

impl SyncWasmPlugins {
    /// Construct an empty sync plugin store bound to the shared bag layer.
    #[must_use]
    pub fn new(
        engine: wasmtime::Engine,
        bags: crate::bag::InstanceBagStore,
        globals: crate::bag::GlobalBagStore,
    ) -> Self {
        Self {
            store: RefCell::new(StoreSet::new(StoreKind::Sync, engine, bags, globals)),
            keybinds: RefCell::new(Vec::new()),
        }
    }

    /// Construct an empty instance with no loaded plugins, for tests.
    #[must_use]
    pub fn empty() -> Self {
        use wasmtime::Config;
        let engine = wasmtime::Engine::new(&Config::new()).expect("wasmtime engine");
        Self::new(
            engine,
            crate::bag::InstanceBagStore::default(),
            crate::bag::GlobalBagStore::default(),
        )
    }

    /// All keybinds declared by loaded plugins' manifests.
    #[must_use]
    pub fn declared_keybinds(&self) -> Vec<crate::loader::ManifestKeybind> {
        self.keybinds.borrow().clone()
    }

    /// Replace the cached keybinds (called by the wiring layer after loading
    /// instances and reading their manifests).
    pub fn set_keybinds(&self, keybinds: Vec<crate::loader::ManifestKeybind>) {
        *self.keybinds.borrow_mut() = keybinds;
    }
    /// Attach the host-import callbacks (emit/request/cancel) that every
    /// subsequently-loaded instance's `StoreState` will carry.
    pub fn set_imports(&mut self, imports: crate::imports::HostImports) {
        self.store.get_mut().set_imports(imports);
    }

    pub fn store_mut(&mut self) -> &mut StoreSet {
        self.store.get_mut()
    }

    /// Load global plugins into the sync store and register their declared
    /// keybinds from the manifests returned by `get-manifest()`.
    ///
    /// Called once at startup by the wiring layer after host imports are set.
    pub fn load_globals(
        &mut self,
        plugins: &[crate::loader::CompiledPlugin],
        linker: &wasmtime::component::Linker<crate::store::StoreState>,
    ) -> Result<(), error_stack::Report<crate::loader::PluginLoadError>> {
        let mut keybinds = Vec::new();
        for plugin in plugins {
            if plugin.meta.kind != crate::discovery::PluginKind::Global {
                continue;
            }
            let ctx = crate::store::InstanceCtx {
                plugin_name: plugin.meta.name.clone(),
                instance_id: crate::loader::synthetic_global_id(&plugin.meta.name),
                session_id: None,
            };
            match self.store.get_mut().load(&plugin.component, ctx, linker) {
                Err(e) => {
                    tracing::warn!(?e, name = %plugin.meta.name, "failed to load global plugin into sync store");
                }
                Ok(manifest) => {
                    let cached = crate::loader::convert_manifest(&plugin.meta.name, manifest);
                    keybinds.extend(cached.keybinds.clone());
                }
            }
        }
        self.set_keybinds(keybinds);
        Ok(())
    }

    /// Number of loaded instances.
    #[must_use]
    pub fn len(&self) -> usize {
        self.store.borrow().len()
    }

    /// Whether no instances are loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.store.borrow().is_empty()
    }
}

impl jinn_domain::feat::plugin_dispatch::PluginSyncHooks for SyncWasmPlugins {
    fn call_hooks(&self, hook: &str, ctx: &HookContext) -> Vec<Value> {
        let mut store = self.store.borrow_mut();
        store
            .iter_mut()
            .filter_map(|inst| {
                let plugin_name = inst.ctx().plugin_name.clone();
                match crate::dispatch::dispatch_sync_hook(inst, hook, ctx.value()) {
                    Ok(Some(v)) => Some(v),
                    Ok(None) => None,
                    Err(e) => {
                        tracing::error!(
                            hook, plugin = %plugin_name, error = %e,
                            "sync plugin hook failed"
                        );
                        None
                    }
                }
            })
            .collect()
    }
}
