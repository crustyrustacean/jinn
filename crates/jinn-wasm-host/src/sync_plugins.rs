//! Render-thread sync plugins — direct hook calls on the sync store set.
//!
//! [`SyncWasmPlugins`] is `!Send` because `wasmtime::Store` is `!Send`. It lives
//! on the render thread and calls sync hook exports (`on-chat-input-badges-render`,
//! `on-keybind-trigger`, `on-submit-intercept`) directly, with zero channel hops.
//! This mirrors the old `jinn_plugin::SyncPlugins` which owned a `!Send` Lua state.
//!
//! The trait keeps the old `serde_json::Value` shape (`PluginSyncHooks`);
//! internally each call builds a `Val::Record` from the ctx JSON, calls the
//! export by name (runtime export lookup — absent exports skipped), and converts
//! the result back to JSON.

use std::cell::RefCell;

use serde_json::Value;
use wasmtime::component::Val;

use jinn_core_types::SessionId;
use jinn_domain::feat::plugin_dispatch::{HookContext, PluginHookSite};

use crate::store::{InstanceCtx, StoreKind, StoreSet};
use crate::val_convert::{json_to_val, val_to_json};

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
    /// Mutable access to the underlying store set (for the wiring layer to load
    /// instances + attach host imports).
    pub fn store_mut(&mut self) -> &mut StoreSet {
        self.store.get_mut()
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
                inst.with(|store, instance| call_one_hook(store, instance, hook, ctx, &plugin_name))
            })
            .collect()
    }
}

/// Call a single hook export by name, converting the result back to JSON.
///
/// Absent exports return `None` (optional-hook semantics). Hook traps are
/// logged and dropped — a buggy plugin degrades rather than panicking the
/// render thread.
fn call_one_hook(
    store: &mut wasmtime::Store<crate::store::StoreState>,
    instance: &wasmtime::component::Instance,
    hook: &str,
    ctx: &HookContext,
    plugin_name: &str,
) -> Option<Value> {
    let Some(func) = instance.get_func(&mut *store, hook) else {
        return None;
    };
    let param = json_to_val(ctx.value());
    let params = vec![param];
    let mut results = vec![Val::Bool(false)];
    match func.call(&mut *store, &params, &mut results) {
        Ok(()) => {
            let v = results.first().map(val_to_json).unwrap_or(Value::Null);
            (!v.is_null()).then_some(v)
        }
        Err(e) => {
            let report = error_stack::Report::new(SyncHookError)
                .attach(e.to_string())
                .attach(PluginHookSite {
                    plugin: plugin_name.to_owned(),
                    hook: hook.to_owned(),
                });
            tracing::error!(hook, error = ?report, "sync plugin hook failed");
            None
        }
    }
}

/// Build a per-instance ctx from the plugin's identity.
#[allow(dead_code)]
fn instance_ctx(plugin_name: &str, session_id: Option<SessionId>) -> InstanceCtx {
    InstanceCtx {
        plugin_name: plugin_name.to_owned(),
        instance_id: jinn_core_types::PluginInstanceId::new(),
        session_id,
    }
}
