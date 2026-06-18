//! Stable unique identifier for a single attached-plugin *instance*.
//!
//! Generated when an [`PluginInstanceId`] is created and persisted with it, so
//! the identity survives restarts. Two attachments of the same plugin name get
//! distinct ids. Old persisted data lacking the field hydrates a fresh id via
//! `#[serde(default)]`.
//!
//! Stored as an opaque string and derives equality/hashing so it can be used
//! as a `HashMap` key (the per-session hooks map and the plugin-data store both
//! key on it).
//!
//! Lives in `jinn-core-types` (rather than the plugin engine) because it appears
//! in the `PluginFire` trait signature defined in the host (`jinn-domain`).
//! Both host and engine reference it here without a cycle.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginInstanceId(String);

impl PluginInstanceId {
    /// Generate a new unique instance id using UUID v7.
    #[must_use]
    pub fn new() -> Self {
        Self(format!("i-{}", Uuid::now_v7()))
    }
}

impl Default for PluginInstanceId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<String> for PluginInstanceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for PluginInstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test file, panics are acceptable"
    )]
    use super::*;

    #[rstest::rstest]
    fn plugin_instance_id_new_is_unique() {
        let a = PluginInstanceId::new();
        let b = PluginInstanceId::new();
        assert_ne!(a, b);
    }

    #[rstest::rstest]
    fn plugin_instance_id_serializes_with_prefix() {
        let id = PluginInstanceId::new();
        let json = serde_json::to_string(&id).expect("serialize");
        assert!(json.contains("i-"));
    }
}
