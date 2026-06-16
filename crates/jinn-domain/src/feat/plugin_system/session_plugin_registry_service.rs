//! Service wrapper for [`SessionPluginRegistry`].
//!
//! Same shape as `PluginFireService` — cheap-to-clone opaque wrapper around
//! `Arc<dyn SessionPluginRegistry>`, with `#[derive(Debug, Clone)]` via
//! `derive_more`.

use std::sync::Arc;

use derive_more::Debug;
use error_stack::Report;

use crate::feat::plugin_system::{
    CreateSessionRegistryResult, SessionPluginRegistry, SessionPluginRegistryError,
    SessionRegistryId,
};
use crate::protocol::SessionId;
/// Service wrapper for [`SessionPluginRegistry`].
///
/// Cheap to clone (Arc). Construct once at startup, share via [`crate::Services`].
#[derive(Debug, Clone)]
pub struct SessionPluginRegistryService {
    #[debug("SessionPluginRegistry<{}>", self.backend.name())]
    backend: Arc<dyn SessionPluginRegistry>,
}

impl SessionPluginRegistryService {
    /// Construct a new service wrapper around a [`SessionPluginRegistry`] backend.
    #[must_use]
    pub fn new(backend: Arc<dyn SessionPluginRegistry>) -> Self {
        Self { backend }
    }

    /// Create a per-session Lua state with the named plugins loaded.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is unreachable or any named plugin
    /// cannot be loaded.
    pub async fn create_session_registry(
        &self,
        instances: Vec<(crate::feat::plugin_system::PluginInstanceId, String)>,
        origin_session_id: SessionId,
    ) -> Result<CreateSessionRegistryResult, Report<SessionPluginRegistryError>> {
        self.backend
            .create_session_registry(instances, origin_session_id)
            .await
    }

    /// Drop a per-session Lua state.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead.
    pub async fn destroy_session_registry(
        &self,
        registry_id: SessionRegistryId,
    ) -> Result<(), Report<SessionPluginRegistryError>> {
        self.backend.destroy_session_registry(registry_id).await
    }
}
