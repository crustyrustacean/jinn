//! Plugin system for jinn.
//!
//! Two execution contexts, same scripts, four access patterns:
//!
//! | Who            | Return values? | Blocking?            | API                                                    |
//! |----------------|----------------|----------------------|--------------------------------------------------------|
//! | Render thread  | Yes            | No (direct Lua call) | `app.plugins.sync_hooks("name")` → lazy iterator      |
//! | Actor          | No             | No (async)           | `services.plugins.fire_async("name", &ctx)`             |
//! | Actor          | Yes            | No (async)           | `services.plugins.fire_async_collect("name", &ctx)`→Vec |
//! | Actor          | Yes            | Yes (blocking)       | `services.plugin_sync.call_hooks("name", &ctx)` → Vec    |
//!
//! - **Sync** — render thread, hooks return immediately via [`SyncPlugins`]
//! - **Async** — background thread, hooks can call `ctx.request()` via [`AsyncPluginHandle`]

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── SessionRegistryId (was in plugin_system before merge) ──────────

/// Opaque identifier for a per-session plugin registry on the plugin thread.
///
/// Created by the plugin system when a session attaches plugins and stored
/// on the session. Passed back to the plugin system when firing session-scoped
/// hooks via `PluginFire::fire_async_for_session` or
/// `PluginSyncCall::call_hooks_for_session`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionRegistryId(Uuid);

impl SessionRegistryId {
    /// Generate a new random registry ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionRegistryId {
    fn default() -> Self {
        Self::new()
    }
}

// ── Module declarations ────────────────────────────────────────────

pub mod async_handle;
pub mod async_thread;
pub mod bindings;
pub mod command;
pub mod in_flight_requests;
pub mod loader;
pub mod plugin_data;
pub mod plugin_fire_impl;
pub mod plugin_sync_impl;
pub mod session_plugin_registry;
pub mod session_plugin_registry_service;
pub mod session_registry;
pub mod sync_handle;
pub mod sync_state;
pub mod system;
pub mod tool_def;

// ── Re-exports ─────────────────────────────────────────────────────

pub use crate::feat::attached_plugin::PluginInstanceId;
pub use async_handle::AsyncPluginHandle;
pub use async_thread::RequestHandler;
pub use command::PluginCommand;
pub use in_flight_requests::InFlightRequests;
pub use loader::{PluginKind, PluginMeta, discover_plugins};
pub use plugin_data::PluginData;
pub use session_plugin_registry::{
    CreateSessionRegistryResult, PluginToolMetadata, SessionPluginRegistry,
    SessionPluginRegistryError, ToolScope,
};
pub use session_plugin_registry_service::SessionPluginRegistryService;
pub use sync_handle::PluginSyncHandle;
pub use sync_state::{PluginHooks, SyncPlugins};
pub use system::{CommandDispatcher, PluginSystem, PluginSystemBuildResult};
pub use tool_def::{PluginToolDef, PluginToolMetadata as ToolMeta, ToolScope as ToolScopeReexport};

/// A no-op request handler for contexts where async requests aren't needed.
#[must_use]
pub fn noop_request_handler() -> RequestHandler {
    std::sync::Arc::new(
        |name: &str,
         _data: &serde_json::Value,
         _cancel: Option<tokio_util::sync::CancellationToken>| {
            tracing::warn!(name, "no request handler configured, returning null");
            std::boxed::Box::pin(async { serde_json::Value::Null })
        },
    )
}

/// A no-op command dispatcher for test contexts.
#[must_use]
pub fn noop_command_dispatcher() -> CommandDispatcher {
    std::sync::Arc::new(|cmd: PluginCommand| {
        tracing::warn!(
            name = cmd.name,
            "no command dispatcher configured, dropping"
        );
    })
}
