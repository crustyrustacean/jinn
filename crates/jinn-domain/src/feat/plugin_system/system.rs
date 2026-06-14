//! Plugin system construction — entry point for the entire plugin system.
//!
//! [`PluginSystem::build`] discovers plugins, loads them into two Lua states
//! (sync + async), spawns the background thread and drainer tasks, and
//! returns three handles.

use std::path::Path;
use std::sync::Arc;

use super::async_handle::PluginJob;
use super::async_thread::{RequestHandler, run_async_thread};
use super::command::PluginCommand;
use super::loader::{PluginMeta, discover_plugins, load_all};
use super::plugin_data::PluginData;
use super::sync_state::SyncPlugins;
use super::tool_def::PluginToolMetadata;
use super::{async_handle::AsyncPluginHandle, sync_handle::PluginSyncHandle};

/// Result of [`PluginSystem::build`] — all handles and metadata.
pub struct PluginSystemBuildResult {
    /// Sync plugin state for render-thread hook calls.
    pub sync: SyncPlugins,
    /// Async handle for domain-layer plugin operations.
    pub async_handle: AsyncPluginHandle,
    /// Sync handle for test/threaded hook calls.
    pub sync_handle: PluginSyncHandle,
    /// Tool definitions extracted from global plugins.
    pub global_tool_metadata: Vec<PluginToolMetadata>,
}

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
        clippy::needless_pass_by_value,
        reason = "trait-object wrappers and runtime handle are intentionally moved into the plugin system"
    )]
    pub fn build(
        user_dir: &Path,
        system_dir: &Path,
        runtime_handle: tokio::runtime::Handle,
        command_dispatcher: CommandDispatcher,
        request_handler: RequestHandler,
    ) -> PluginSystemBuildResult {
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

        // All discovered plugins load into the sync Lua state so that sync hooks
        // (e.g. on_session_preview) work for attachable plugins too.
        // Async-only hooks (on_turn_end) are never called from the sync path.
        // The async thread loads globals at startup and attachable on-demand per-session.

        let global_plugins: Vec<PluginMeta> = plugins
            .iter()
            .filter(|m| m.kind == super::loader::PluginKind::Global)
            .cloned()
            .collect();

        // Load into sync Lua state.
        let sync_lua = mlua::Lua::new();
        let sync_result = load_all(&sync_lua, &plugins);

        // Async channel: async fire → background thread.
        let (job_tx, job_rx) = kanal::unbounded_async::<PluginJob>();

        // Pass *all* plugins to the async thread: globals are preloaded into
        // the shared Lua state, attachable metas are kept for on-demand
        // per-session loading via `PluginJob::LoadSession`.
        let async_plugins = plugins;
        let async_global_plugins = global_plugins;
        let async_plugin_data = plugin_data.clone();
        let async_emit_tx = emit_tx.clone_async();
        let async_request_handler = request_handler.clone();
        let in_flight = super::InFlightRequests::new();
        let async_in_flight = in_flight.clone();

        #[expect(
            clippy::expect_used,
            reason = "thread spawn failure is fatal — see `# Panics`"
        )]
        std::thread::Builder::new()
            .name("plugin-async".to_owned())
            .spawn(move || {
                let async_lua = mlua::Lua::new();
                let async_result = load_all(&async_lua, &async_global_plugins);
                run_async_thread(
                    job_rx,
                    async_lua,
                    async_result.hooks,
                    async_result.tools,
                    async_plugins,
                    async_plugin_data,
                    async_emit_tx,
                    async_request_handler,
                    async_in_flight,
                );
            })
            .expect("spawn plugin-async thread");

        let sync = SyncPlugins::new(
            sync_lua,
            sync_result.hooks,
            plugin_data.clone(),
            emit_tx,
            in_flight.clone(),
        );

        let async_handle = AsyncPluginHandle::new(job_tx.clone(), plugin_data);

        // clone_sync() preserves the async sender while creating a new sync
        // sender sharing the same channel internal.
        let sync_handle = PluginSyncHandle::new(job_tx.clone_sync());

        PluginSystemBuildResult {
            sync,
            async_handle,
            sync_handle,
            global_tool_metadata: sync_result
                .tools
                .iter()
                .map(super::tool_def::PluginToolDef::to_metadata)
                .collect(),
        }
    }
}

/// Convenience: discover plugin metadata.
///
/// Returns the discovered plugin list for inspection (e.g., sidebar display).
#[must_use]
pub fn discover_from_paths(user_dir: &Path, system_dir: &Path) -> Vec<PluginMeta> {
    discover_plugins(user_dir, system_dir)
}
