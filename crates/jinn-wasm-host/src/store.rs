//! Dual-store model — the architectural invariant.
//!
//! `wasmtime::Store` is `!Send`: it cannot move between threads. But jinn
//! fires hooks from two threads:
//!
//! 1. The **async host thread** (inside a `tokio::spawn`/`LocalSet`):
//!    lifecycle hooks, LLM callbacks, plugin-defined async hooks.
//! 2. The **render thread** (the TUI): sync render hooks (`badges`,
//!    `keybind-trigger`, `submit-intercept`).
//!
//! A component exporting hooks fired on *both* threads must be instantiated
//! twice — once per store — against the **same** compiled component bytes and
//! the **same** host-owned bag layer. This mirrors the old Lua system's two
//! Lua states (sync + async) sharing one `PluginData`/`GlobalPluginData`.
//!
//! # Storage model
//!
//! Each loaded instance keeps its own `Store<StoreState>` alive alongside its
//! `Instance` (a wasmtime `Instance` is only valid while its `Store` lives).
//! They are co-owned by a [`StoredInstance`] and co-borrowed on every hook
//! call.
//!
//! Both store sets (async + sync) share one `InstanceBagStore` and one
//! `GlobalBagStore` via `Arc`, so a write on one thread is visible to the
//! other on its next read — same visibility contract as the Lua system.

use std::collections::HashMap;

use error_stack::ResultExt;
use wasmtime::component::Instance;
use wasmtime::{Engine, Store};

use crate::bag::{GlobalBagStore, InstanceBagStore};
use crate::engine::CompiledComponent;

/// Which thread a store set belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    /// The async host thread (lifecycle, LLM, plugin-defined async hooks).
    Async,
    /// The render thread (sync render hooks).
    Sync,
}

/// Per-instance identity held inside a `Store`'s host state (`StoreState`).
///
/// The host imports read this from the active store context to route bag
/// access to the right slot.
#[derive(Debug, Clone)]
pub struct InstanceCtx {
    /// Plugin name (for global bag keying + logging).
    pub plugin_name: String,
    /// Instance id (for attached bag keying). Global plugins use a synthetic.
    pub instance_id: jinn_core_types::PluginInstanceId,
    /// Session this instance is attached to (`None` for global plugins).
    pub session_id: Option<jinn_core_types::SessionId>,
}

/// A compiled instance kept alive with its owning `Store`.
///
/// The `Store`, `Instance`, and typed-export indices are co-owned and
/// co-borrowed on every hook call via [`StoredInstance::with`] /
/// [`StoredInstance::with_async`]. The `GuestIndices` resolve the typed
/// `hooks` exports (well-known + `run-trigger`/`run-tool`) for dispatch;
/// holding the `InstancePre` keeps those indices valid.
pub struct StoredInstance {
    pub ctx: InstanceCtx,
    store: Store<StoreState>,
    instance: Instance,
    instance_pre: wasmtime::component::InstancePre<StoreState>,
    indices: crate::bindings::exports::jinn::plugin::hooks::GuestIndices,
}

impl StoredInstance {
    /// Borrow the store + instance together for a sync export call.
    pub fn with<R>(&mut self, f: impl FnOnce(&mut Store<StoreState>, &Instance) -> R) -> R {
        f(&mut self.store, &self.instance)
    }

    /// Borrow store + instance for an async export call. The closure returns a
    /// Future that borrows the store for the await duration.
    pub async fn with_async<'a, F, Fut, R>(&'a mut self, f: F) -> R
    where
        F: FnOnce(&'a mut Store<StoreState>, &'a Instance) -> Fut,
        Fut: std::future::Future<Output = R> + 'a,
    {
        f(&mut self.store, &self.instance).await
    }

    /// Resolve the typed `hooks` exports for this instance.
    pub fn typed_guest(
        &mut self,
    ) -> wasmtime::Result<crate::bindings::exports::jinn::plugin::hooks::Guest> {
        self.indices.load(&mut self.store, &self.instance)
    }

    /// Borrow the underlying store mutably (for run_concurrent callers).
    pub fn store_mut(&mut self) -> &mut Store<StoreState> {
        &mut self.store
    }

    /// The instance identity.
    #[must_use]
    pub fn ctx(&self) -> &InstanceCtx {
        &self.ctx
    }
}

/// A set of component instances on one thread's store.
///
/// `!Send`. Constructed and used on a single thread. The async store set
/// holds async-capable instances; the sync store set holds sync instances.
/// Both share the bag stores (passed in by `Arc`-clone).
pub struct StoreSet {
    kind: StoreKind,
    engine: Engine,
    instances: HashMap<InstanceKey, StoredInstance>,
    bags: InstanceBagStore,
    globals: GlobalBagStore,
    imports: Option<crate::imports::HostImports>,
}

impl std::fmt::Debug for StoreSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreSet")
            .field("kind", &self.kind)
            .field("instances", &self.instances.len())
            .finish_non_exhaustive()
    }
}

/// Identity key for a loaded instance within a store.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InstanceKey {
    plugin_name: String,
    instance_id: jinn_core_types::PluginInstanceId,
}

/// Error loading a component instance.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct StoreLoadError;

impl StoreSet {
    /// Construct an empty store set bound to one engine + shared bag layer.
    ///
    /// `bags` / `globals` are shared across both store sets (cloned by `Arc`).
    pub fn new(
        kind: StoreKind,
        engine: Engine,
        bags: InstanceBagStore,
        globals: GlobalBagStore,
    ) -> Self {
        Self {
            kind,
            engine,
            instances: HashMap::new(),
            bags,
            globals,
            imports: None,
        }
    }
    /// Attach the host-import callbacks (emit/request/cancel) that every
    /// subsequently-loaded instance's `StoreState` will carry.
    pub fn set_imports(&mut self, imports: crate::imports::HostImports) {
        self.imports = Some(imports);
    }

    /// Which thread this set belongs to.
    #[must_use]
    pub fn kind(&self) -> StoreKind {
        self.kind
    }

    /// Instantiate a compiled component under a given instance identity.
    ///
    /// The linker is provided by the caller, already wired to this store
    /// set's bag layer and request/emit closures.
    ///
    /// # Errors
    ///
    /// Returns an error if instantiation fails (missing imports, trap, etc.).
    pub fn load(
        &mut self,
        component: &CompiledComponent,
        ctx: InstanceCtx,
        linker: &wasmtime::component::Linker<StoreState>,
        runtime: &tokio::runtime::Handle,
    ) -> Result<
        crate::bindings::exports::jinn::plugin::hooks::Manifest,
        error_stack::Report<StoreLoadError>,
    > {
        let mut store = Store::new(
            &self.engine,
            StoreState::new(ctx.clone(), &self.bags, &self.globals, self.imports.clone()),
        );
        let instance_pre = linker
            .instantiate_pre(component.inner())
            .map_err(|e| error_stack::Report::new(StoreLoadError).attach(e.to_string()))
            .attach("building instance pre for typed export indices")?;
        let indices =
            crate::bindings::exports::jinn::plugin::hooks::GuestIndices::new(&instance_pre)
                .map_err(|e| error_stack::Report::new(StoreLoadError).attach(e.to_string()))
                .attach("resolving typed hooks export indices")?;
        // Components with async exports require the `*_async` instantiation
        // path. The caller's runtime is multi-threaded, so `block_in_place`
        // lets us synchronously drive the async instantiation without nesting
        // runtimes. Startup only — never called per-hook.
        let instance = tokio::task::block_in_place(|| {
            runtime.block_on(instance_pre.instantiate_async(&mut store))
        })
        .map_err(|e| error_stack::Report::new(StoreLoadError).attach(e.to_string()))
        .attach("instantiating wasm component")?;
        let key = InstanceKey {
            plugin_name: ctx.plugin_name.clone(),
            instance_id: ctx.instance_id.clone(),
        };
        self.instances.insert(
            key.clone(),
            StoredInstance {
                ctx,
                store,
                instance,
                instance_pre,
                indices,
            },
        );
        // Keybinds are not read here. The async host thread reads every
        // plugin's manifest at startup; the wiring layer pushes those keybinds
        // into this sync store via `set_keybinds`. The sync (render-thread)
        // store only needs instances for firing sync render hooks.
        let _ = self.instances.get_mut(&key).expect("just inserted");
        let default_manifest = crate::bindings::exports::jinn::plugin::hooks::Manifest {
            description: None,
            keybinds: vec![],
            tools: vec![],
        };
        Ok(default_manifest)
    }

    /// Async variant for store sets configured with async support
    /// (`StoreKind::Async`). wasmtime requires `*_async` instantiation +
    /// `call_*_async` when the engine config has async support enabled.
    pub async fn load_async(
        &mut self,
        component: &CompiledComponent,
        ctx: InstanceCtx,
        linker: &wasmtime::component::Linker<StoreState>,
    ) -> Result<
        crate::bindings::exports::jinn::plugin::hooks::Manifest,
        error_stack::Report<StoreLoadError>,
    > {
        let mut store = Store::new(
            &self.engine,
            StoreState::new(ctx.clone(), &self.bags, &self.globals, self.imports.clone()),
        );
        let instance_pre = linker
            .instantiate_pre(component.inner())
            .map_err(|e| error_stack::Report::new(StoreLoadError).attach(e.to_string()))
            .attach("building instance pre for typed export indices")?;
        let indices =
            crate::bindings::exports::jinn::plugin::hooks::GuestIndices::new(&instance_pre)
                .map_err(|e| error_stack::Report::new(StoreLoadError).attach(e.to_string()))
                .attach("resolving typed hooks export indices")?;
        let instance = linker
            .instantiate_async(&mut store, component.inner())
            .await
            .map_err(|e| error_stack::Report::new(StoreLoadError).attach(e.to_string()))
            .attach("instantiating wasm component (async)")?;
        let key = InstanceKey {
            plugin_name: ctx.plugin_name.clone(),
            instance_id: ctx.instance_id.clone(),
        };
        self.instances.insert(
            key.clone(),
            StoredInstance {
                ctx,
                store,
                instance,
                instance_pre,
                indices,
            },
        );
        let inst = self.instances.get_mut(&key).expect("just inserted");
        let guest = inst
            .typed_guest()
            .map_err(|e| error_stack::Report::new(StoreLoadError).attach(e.to_string()))
            .attach("resolving typed guest for get-manifest")?;
        // Sync export on an async-configured store: the generated sync
        // `call_get_manifest` would panic, so invoke the typed func via
        // `call_async` directly.
        let manifest = {
            let f = guest.func_get_manifest();
            let (manifest,) = f
                .call_async(inst.store_mut(), ())
                .await
                .map_err(|e| error_stack::Report::new(StoreLoadError).attach(e.to_string()))
                .attach("calling get-manifest after instantiation")?;
            manifest
        };
        Ok(manifest)
    }

    /// Borrow the instance for a given identity, if loaded in this store set.
    pub fn get_mut(
        &mut self,
        plugin_name: &str,
        instance_id: &jinn_core_types::PluginInstanceId,
    ) -> Option<&mut StoredInstance> {
        let key = InstanceKey {
            plugin_name: plugin_name.to_owned(),
            instance_id: instance_id.clone(),
        };
        self.instances.get_mut(&key)
    }

    /// Iterate all loaded instances mutably (for fan-out hook firing).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut StoredInstance> {
        self.instances.values_mut()
    }

    /// Number of loaded instances.
    #[must_use]
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// Whether the set holds no instances.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }
}

/// Host state stored inside each `wasmtime::Store<T>`.
///
/// Cloned-by-`Arc` references to the shared bag layer; plus the identity of
/// the instance so host imports know which bag slot to touch. Host imports
/// receive `&mut StoreState` (via wasmtime's `Caller<T>`), read `ctx` to
/// identify the slot, and read/write the bags.
#[derive(Debug, Clone)]
pub struct StoreState {
    pub ctx: InstanceCtx,
    pub bags: InstanceBagStore,
    pub globals: GlobalBagStore,
    /// Injected host behaviours (`emit`, `request-*`, etc.). `None` only in
    /// tests / before wiring; production always sets it.
    pub imports: Option<crate::imports::HostImports>,
}

impl wasmtime::component::HasData for StoreState {
    type Data<'a> = &'a mut StoreState;
}

impl StoreState {
    fn new(
        ctx: InstanceCtx,
        bags: &InstanceBagStore,
        globals: &GlobalBagStore,
        imports: Option<crate::imports::HostImports>,
    ) -> Self {
        Self {
            ctx,
            bags: bags.clone(),
            globals: globals.clone(),
            imports,
        }
    }

    /// Attach the host-import callbacks (emit/request/cancel). Called by the
    /// wiring layer once the domain services are available.
    pub fn set_imports(&mut self, imports: crate::imports::HostImports) {
        self.imports = Some(imports);
    }
}
