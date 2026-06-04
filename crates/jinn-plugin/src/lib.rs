//! Plugin system for jinn.
//!
//! Two execution contexts, same scripts:
//! - **Sync** — render thread, hooks return immediately via [`SyncPlugins`]
//! - **Async** — background thread, hooks can call `ctx.request()` via [`AsyncPluginHandle`]
//!
//! Scripts return a table of hooks using the Lua module pattern:
//!
//! ```lua
//! local M = {}
//! function M.on_turn_end(ctx) ... end
//! function M.on_filter_input(ctx) return ctx.text end
//! return M
//! ```
//!
//! Call sites iterate hooks and handle results however they want:
//!
//! ```ignore
//! for hook in plugins.sync_hooks("on_filter_input") {
//!     let result: String = hook.call(&ctx)?;
//! }
//! ```
//!
//! Persistent state lives in [`PluginData`] (an `Arc<DashMap>`), shared
//! between sync and async contexts. Async hooks write via
//! `ctx.set_plugin_data(value)`. Sync hooks read from `ctx.plugin_data`
//! (auto-injected). Call sites never see this.

pub mod async_handle;
pub mod async_thread;
pub mod bindings;
pub mod command;
pub mod loader;
pub mod plugin_data;
pub mod sync_state;
pub mod plugin_fire_impl;
pub mod sync_handle;
pub mod system;

pub use async_handle::AsyncPluginHandle;
pub use sync_handle::PluginSyncHandle;
pub use async_thread::RequestHandler;
pub use command::PluginCommand;
pub use loader::{PluginMeta, discover_plugins};
pub use plugin_data::PluginData;
pub use sync_state::{PluginHooks, SyncPlugins};
pub use system::{CommandDispatcher, PluginSystem};

/// A no-op request handler for contexts where async requests aren't needed.
#[must_use]
pub fn noop_request_handler() -> RequestHandler {
    std::sync::Arc::new(|name: &str, _data: &serde_json::Value| {
        tracing::warn!(name, "no request handler configured, returning null");
        serde_json::Value::Null
    })
}

/// A no-op command dispatcher for test contexts.
#[must_use]
pub fn noop_command_dispatcher() -> CommandDispatcher {
    std::sync::Arc::new(|cmd: PluginCommand| {
        tracing::warn!(name = cmd.name, "no command dispatcher configured, dropping");
    })
}
