//! Session plugin registry trait — abstraction over per-session Lua state lifecycle.
//!
//! `jinn-domain` cannot depend on `jinn-plugin` (circular dependency), so this
//! trait provides the minimal interface for the `PluginDispatchActor` to manage
//! per-session Lua states via the `Services` DI container.

use error_stack::Report;
use wherror::Error;

use crate::feat::plugin_system::SessionRegistryId;

/// Error raised by [`SessionPluginRegistry`] implementations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct SessionPluginRegistryError;

/// Manage per-session plugin Lua states.
///
/// Implemented by `jinn_plugin::AsyncPluginHandle`. The dispatcher uses this
/// trait to spin up isolated Lua states for each session's attached plugins
/// and tear them down on detach.
#[async_trait::async_trait]
pub trait SessionPluginRegistry: Send + Sync {
    /// Create a per-session Lua state with the named attachable plugins loaded.
    ///
    /// Returns an opaque [`SessionRegistryId`] used in subsequent
    /// `PluginFire::fire_async_for_session_json` calls.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is unreachable or any named plugin
    /// cannot be loaded.
    async fn create_session_registry(
        &self,
        plugin_names: Vec<String>,
    ) -> Result<SessionRegistryId, Report<SessionPluginRegistryError>>;

    /// Drop a per-session Lua state.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead.
    async fn destroy_session_registry(
        &self,
        registry_id: SessionRegistryId,
    ) -> Result<(), Report<SessionPluginRegistryError>>;

    /// Returns the name of this backend for debugging.
    fn name(&self) -> &'static str;
}
