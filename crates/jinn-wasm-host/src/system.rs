//! WASM plugin system build — the single entry point that wires everything.
//!
//! This is the WASM analogue of the old `jinn_plugin::PluginSystem::build`.
//! It spawns the dedicated background thread that owns the `!Send` async
//! `StoreSet`, exposes a `Send + Sync + Clone` [`AsyncWasmHandle`] for the
//! domain layer to fire hooks through, and returns discovery metadata.
//!
//! # Architecture
//!
//! `wasmtime::Store` is `!Send`, but jinn fires hooks from two threads:
//!
//! - **async host thread** — lifecycle hooks, LLM callbacks, plugin-defined
//!   async hooks (e.g. `on-enrich`). These run inside a `tokio::LocalSet`
//!   on the dedicated background thread so the `!Send` future can be polled.
//! - **render thread** — sync render hooks (`badges`, `keybind-trigger`,
//!   `submit-intercept`). These never `.await`, so they run on the
//!   render-thread-local sync store set directly.
//!
//! Both store sets read/write the **same** host-owned bag layer (cloned by
//! `Arc`), mirroring the old Lua system's two Lua states sharing one
//! `PluginData`. This is the dual-store invariant.

use std::path::Path;

use error_stack::{Report, ResultExt};

use crate::async_thread::AsyncThreadHandle;
use crate::bag::{GlobalBagStore, InstanceBagStore};
use crate::discovery::{PluginKind, PluginMeta};
use crate::engine::{EngineConfig, WasmEngine};
use crate::handle::AsyncWasmHandle;
use crate::imports::{HostImports, register as register_imports};
use crate::loader::{compile_discovered};

pub use crate::async_thread::AsyncPluginError;
pub use crate::loader::PluginLoadError;

/// Callback that dispatches a plugin `emit(cmd)` to the domain bus.
///
/// Receives the typed WIT command variant and the plugin name; the wiring
/// layer translates the command to domain messages. Mirrors the old
/// `CommandDispatcher = Arc<dyn Fn(PluginCommand)>`.
pub type CommandDispatcher =
    std::sync::Arc<dyn Fn(&str, &crate::bindings::command::Command) + Send + Sync>;

/// The result of building the WASM plugin system.
///
/// Mirrors the old `PluginSystemBuildResult`: the async handle (shared across
/// the app), the cached manifest metadata, and discovered attachable plugins.
pub struct WasmPluginSystem {
    /// `Send + Sync + Clone` handle for firing async hooks from the domain.
    pub async_handle: AsyncWasmHandle,
    /// `Send + Sync + Clone` handle for blocking sync hook calls from actors.
    pub sync_handle: crate::sync_handle::SyncWasmHandle,
    /// Render-thread-local sync store set (`!Send`) for TUI render hooks.
    pub sync_plugins: crate::sync_plugins::SyncWasmPlugins,
    /// Discovered attachable plugin metadata (for the plugin picker UI).
    pub attachable_metas: Vec<PluginMeta>,
}
/// Build the WASM plugin system from on-disk plugins.
///
/// Reads `.wasm` + sidecar `plugin.toml` files from `user_dir` / `system_dir`,
/// compiles them once against a shared engine, spawns the dedicated background
/// thread, and returns the handles. The host-import callbacks (`emit`,
/// `request-*`) are supplied by the wiring layer so the host crate stays
/// decoupled from domain internals.
///
/// # Errors
///
/// Returns an error if the engine cannot be constructed or plugins cannot be
/// read/compiled.
#[allow(clippy::missing_errors_doc)]
pub fn build(
    user_dir: &Path,
    system_dir: &Path,
    runtime_handle: tokio::runtime::Handle,
    host_imports: HostImports,
) -> Result<WasmPluginSystem, Report<PluginLoadError>> {
    let engine = WasmEngine::new(&EngineConfig::default())
        .change_context(PluginLoadError)
        .attach("constructing wasm engine")?;

    let (bags, globals) = (InstanceBagStore::new(), GlobalBagStore::new());

    // Compile all discovered plugins once.
    let plugins = compile_discovered(&engine, user_dir, system_dir)
        .attach("compiling discovered wasm plugins")?;
    tracing::info!(count = plugins.len(), "discovered wasm plugins");

    // Build the shared Linker (imports registered once; reused by both stores).
    let linker = {
        let mut linker = wasmtime::component::Linker::<crate::store::StoreState>::new(engine.inner());
        register_imports(&mut linker, &host_imports)
            .change_context(PluginLoadError)
            .attach("registering wasm host imports into linker")?;
        linker
    };

    // Spawn the async background thread. It owns the async StoreSet (!Send)
    // inside a LocalSet on a dedicated OS thread.
    let (job_tx, sync_tx, _thread_handle) = AsyncThreadHandle::spawn(
        engine.clone(),
        bags.clone(),
        globals.clone(),
        linker,
        runtime_handle,
    );

    let async_handle = AsyncWasmHandle::new(job_tx, bags.clone(), globals.clone());
    let sync_handle = crate::sync_handle::SyncWasmHandle::new(sync_tx);
    let sync_plugins = crate::sync_plugins::SyncWasmPlugins::new(
        engine.inner().clone(),
        bags,
        globals,
    );

    let attachable_metas: Vec<PluginMeta> = plugins
        .iter()
        .filter(|p| p.meta.kind == PluginKind::Attachable)
        .map(|p| p.meta.clone())
        .collect();

    Ok(WasmPluginSystem {
        async_handle,
        sync_handle,
        sync_plugins,
        attachable_metas,
    })
}
