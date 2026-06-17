//! Opaque identifier for a per-session plugin registry on the plugin thread.
//!
//! Created by the plugin system when a session attaches plugins and stored
//! on the session. Passed back to the plugin system when firing session-scoped
//! hooks via `PluginFire::fire_async_for_session` or
//! `PluginSyncCall::call_hooks_for_session`.
//!
//! Lives in `jinn-core-types` (rather than the plugin engine) because it appears
//! in the `PluginFire` / `PluginSyncCall` trait signatures defined in the host
//! (`jinn-domain`). Both host and engine reference it here without a cycle.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn session_registry_id_new_is_unique() {
        let a = SessionRegistryId::new();
        let b = SessionRegistryId::new();
        assert_ne!(a, b);
    }
}
