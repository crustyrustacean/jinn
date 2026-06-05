//! Plugin system construction — entry point for the entire plugin system.
//!
//! [`PluginSystem::new`] discovers plugins, loads them into two Lua states
//! (sync + async), spawns the background thread and drainer tasks, and
//! returns three handles.

use std::path::Path;
use std::sync::Arc;

use crate::async_handle::PluginJob;
use crate::async_thread::{RequestHandler, run_async_thread};
use crate::command::PluginCommand;
use crate::loader::{PluginMeta, discover_plugins, load_all};
use crate::plugin_data::PluginData;
use crate::sync_state::SyncPlugins;
use crate::{AsyncPluginHandle, PluginSyncHandle};

/// Callback type for dispatching commands emitted by plugins.
///
/// Called by the emit drainer task for each `PluginCommand` sent through
/// `ctx.emit()`. The wiring layer provides the concrete implementation
/// that translates command names to typed domain commands.
pub type CommandDispatcher = Arc<dyn Fn(PluginCommand) + Send + Sync>;

/// The plugin system entry point.
///
/// Construct once at startup. Returns three handles:
/// - `SyncPlugins` — `!Send`, lives on the render thread
/// - `AsyncPluginHandle` — `Send + Sync + Clone`, used from async contexts
/// - `PluginSyncHandle` — `Send + Sync + Clone`, used for blocking sync calls from actors
pub struct PluginSystem;

impl PluginSystem {
    /// Construct the plugin system.
    ///
    /// Discovers plugins from `user_dir` and `system_dir` (user overrides
    /// system), loads them into two Lua states, spawns the async background
    /// thread and the emit drainer task.
    ///
    /// # Parameters
    ///
    /// - `user_dir` — user plugins directory (e.g., `~/.config/jinn/plugins`)
    /// - `system_dir` — system plugins directory (e.g., `/usr/share/jinn/plugins`)
    /// - `runtime_handle` — tokio runtime handle for spawning drainer tasks
    /// - `command_dispatcher` — callback for dispatching emitted commands
    /// - `request_handler` — callback for handling `ctx.request()` calls
    ///
    /// # Returns
    ///
    /// A tuple of `(SyncPlugins, AsyncPluginHandle, PluginSyncHandle)`.
    ///
    /// # Panics
    ///
    /// Panics if the OS refuses to spawn the plugin-async thread.
    #[expect(
        clippy::too_many_arguments,
        reason = "construction takes many inputs by design"
    )]
    pub fn new(
        user_dir: &Path,
        system_dir: &Path,
        runtime_handle: tokio::runtime::Handle,
        command_dispatcher: CommandDispatcher,
        request_handler: RequestHandler,
    ) -> (SyncPlugins, AsyncPluginHandle, PluginSyncHandle) {
        let plugin_data = PluginData::new();

        // Emit channel: constructed sync (sync Lua hooks call sync `.send()`),
        // drainer uses async recv via `to_async()`.
        let (emit_tx, emit_rx) = kanal::unbounded::<PluginCommand>();

        // Spawn emit drainer on tokio. Async recv — does not block executor.
        {
            let dispatcher = command_dispatcher.clone();
            runtime_handle.spawn(async move {
                let emit_rx = emit_rx.to_async();
                loop {
                    match emit_rx.recv().await {
                        Ok(cmd) => dispatcher(cmd),
                        Err(_) => {
                            tracing::debug!("plugin emit drainer shutting down");
                            break;
                        }
                    }
                }
            });
        }

        // Discover plugins from disk.
        let plugins = discover_plugins(user_dir, system_dir);
        tracing::info!(count = plugins.len(), "discovered plugins");

        // Partition: globals load at startup into both sync + async states.
        // Attachable plugins are loaded on-demand into per-session async states
        // via `AsyncPluginHandle::create_session_registry`.
        let global_plugins: Vec<PluginMeta> = plugins
            .iter()
            .filter(|m| m.kind == crate::loader::PluginKind::Global)
            .cloned()
            .collect();

        // Load into sync Lua state.
        let sync_lua = mlua::Lua::new();
        let sync_hooks = load_all(&sync_lua, &global_plugins);

        // Async channel: async fire → background thread.
        let (job_tx, job_rx) = kanal::unbounded_async::<PluginJob>();

        // Pass *all* plugins to the async thread: globals are preloaded into
        // the shared Lua state, attachable metas are kept for on-demand
        // per-session loading via `PluginJob::LoadSession`.
        let async_plugins = plugins.clone();
        let async_global_plugins = global_plugins.clone();
        let async_plugin_data = plugin_data.clone();
        let async_emit_tx = emit_tx.clone_async();
        let async_request_handler = request_handler.clone();

        std::thread::Builder::new()
            .name("plugin-async".to_owned())
            .spawn(move || {
                let async_lua = mlua::Lua::new();
                let async_hooks = load_all(&async_lua, &async_global_plugins);
                run_async_thread(
                    job_rx,
                    async_lua,
                    async_hooks,
                    async_plugins,
                    async_plugin_data,
                    async_emit_tx,
                    async_request_handler,
                );
            })
            .expect("spawn plugin-async thread");

        let sync = SyncPlugins {
            lua: sync_lua,
            hooks: sync_hooks,
            plugin_data,
            emit_tx,
        };

        let async_handle = AsyncPluginHandle {
            tx: job_tx.clone(),
            plugin_data: sync.plugin_data.clone(),
        };

        // clone_sync() preserves the async sender while creating a new sync
        // sender sharing the same channel internal.
        let sync_handle = PluginSyncHandle {
            tx: job_tx.clone_sync(),
        };

        (sync, async_handle, sync_handle)
    }
}

/// Convenience: discover plugin metadata.
///
/// Returns the discovered plugin list for inspection (e.g., sidebar display).
#[must_use]
pub fn discover_from_paths(user_dir: &Path, system_dir: &Path) -> Vec<PluginMeta> {
    discover_plugins(user_dir, system_dir)
}
