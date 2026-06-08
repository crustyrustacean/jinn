//! Shared plugin data store — cross-context persistent state for plugins.
//!
//! Plugins that need to share data between async and sync hooks use this
//! store, which is live across all clones (`Arc<DashMap>`).
//!
//! - Async hooks write via `ctx.set_plugin_data(value)` (full replace) or
//!   `ctx.merge_plugin_data(value)` (shallow top-level merge), and read
//!   **current** state via `ctx.get_plugin_data()` (re-reads the store; use
//!   after an `await` to observe writes from other fires).
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

    /// Shallow-merge a partial object into a plugin's data.
    ///
    /// Top-level keys in `partial` overwrite the same keys in the stored
    /// value; other top-level keys are untouched. The stored value is
    /// treated as `{}` when no data exists yet, or when the stored value
    /// is not an object (a stray scalar is replaced wholesale). Nested
    /// objects in `partial` replace — are not deep-merged into — the
    /// corresponding nested objects in the stored value.
    pub fn merge(&self, plugin_name: &str, partial: serde_json::Value) {
        use serde_json::Value;

        let merged = self
            .0
            .get(plugin_name)
            .filter(|v| v.is_object())
            .map(|v| {
                let mut current = v.clone();
                if let (Value::Object(cur), Value::Object(part)) = (&mut current, &partial) {
                    for (k, v) in part {
                        cur.insert(k.clone(), v.clone());
                    }
                }
                current
            })
            .unwrap_or(partial);
        self.0.insert(plugin_name.to_owned(), merged);
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

    #[test]
    fn merge_overwrites_top_level_keys_and_keeps_others() {
        // Given a plugin with two fields stored.
        let data = PluginData::new();
        data.set(
            "alpha".to_owned(),
            serde_json::json!({"status": "idle", "count": 3}),
        );

        // When shallow-merging one top-level key.
        data.merge("alpha", serde_json::json!({"status": "enriching"}));

        // Then the merged key updates and the untouched key survives.
        let result = data.get("alpha").expect("should exist");
        assert_eq!(result["status"], "enriching");
        assert_eq!(result["count"], 3);
    }
}
