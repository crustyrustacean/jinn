//! Shared plugin data store — cross-context persistent state for plugins.
//!
//! Plugins that need to share data between async and sync hooks use this
//! store, which is live across all clones (`Arc<DashMap>`).
//!
//! - Async hooks write via `ctx.set_plugin_data(value)` and read **current**
//!   state via `ctx.get_plugin_data()` (re-reads the store; use after an
//!   `await` to observe writes from other fires).
//! - Sync hooks read from `ctx.plugin_data` (auto-injected before each call;
//!   already current at entry since sync hooks never `await`).
//!
//! Keyed by plugin name. Each plugin sees only its own data.

use dashmap::DashMap;
use std::sync::Arc;

/// Thread-safe shared data store for plugins.
///
/// Wrapped in `Arc` so it can be cloned cheaply and shared between
/// the sync (`SyncPlugins`) and async (`AsyncPluginHandle`) halves
/// of the plugin system.
#[derive(Clone)]
pub struct PluginData(Arc<DashMap<String, serde_json::Value>>);

impl PluginData {
    /// Create a new empty plugin data store.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Get a snapshot of a plugin's data, if it exists.
    ///
    /// Returns a cloned `serde_json::Value` — the snapshot is taken
    /// at the moment of the call and won't reflect subsequent writes.
    #[must_use]
    pub fn get(&self, plugin_name: &str) -> Option<serde_json::Value> {
        self.0.get(plugin_name).map(|v| v.clone())
    }

    /// Set a plugin's data. Replaces any previous value.
    pub fn set(&self, plugin_name: String, value: serde_json::Value) {
        self.0.insert(plugin_name, value);
    }
}

impl Default for PluginData {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code"
    )]

    use super::*;

    #[test]
    fn get_returns_none_for_missing_plugin() {
        let data = PluginData::new();
        assert!(data.get("nonexistent").is_none());
    }

    #[test]
    fn set_then_get_roundtrips() {
        let data = PluginData::new();
        data.set("alpha".to_owned(), serde_json::json!({ "verdict": "pass" }));
        let result = data.get("alpha").expect("should exist");
        assert_eq!(result["verdict"], "pass");
    }

    #[test]
    fn set_overwrites_previous_value() {
        let data = PluginData::new();
        data.set("beta".to_owned(), serde_json::json!("first"));
        data.set("beta".to_owned(), serde_json::json!("second"));
        assert_eq!(data.get("beta"), Some(serde_json::json!("second")));
    }

    #[test]
    fn plugins_have_isolated_data() {
        let data = PluginData::new();
        data.set("alpha".to_owned(), serde_json::json!(1));
        data.set("beta".to_owned(), serde_json::json!(2));
        assert_eq!(data.get("alpha"), Some(serde_json::json!(1)));
        assert_eq!(data.get("beta"), Some(serde_json::json!(2)));
    }

    #[test]
    fn clone_shares_underlying_storage() {
        let data = PluginData::new();
        let cloned = data.clone();
        data.set("shared".to_owned(), serde_json::json!("hello"));
        assert_eq!(cloned.get("shared"), Some(serde_json::json!("hello")));
    }
}
