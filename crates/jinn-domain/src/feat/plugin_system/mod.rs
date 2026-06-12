//! Plugin system types shared across crates.
//!
//! This module lives in `jinn-domain` so that both the domain layer and
//! `jinn-plugin` can reference [`SessionRegistryId`] without a circular
//! dependency.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

pub mod session_plugin_registry;

pub use session_plugin_registry::{
    CreateSessionRegistryResult, PluginToolMetadata, SessionPluginRegistry,
    SessionPluginRegistryError, ToolScope,
};

pub mod session_plugin_registry_service;

pub use session_plugin_registry_service::SessionPluginRegistryService;
