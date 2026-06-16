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
//! Keyed per **instance**. Attached plugins key on `(SessionId, PluginInstanceId)`
//! so duplicate instances of the same plugin name get isolated slots. Global
//! plugins (no instance) key on their name.

use crate::SessionId;
use dashmap::DashMap;
use std::sync::Arc;

use super::PluginInstanceId;

/// Composite key for the plugin data store.
///
/// Attached plugins key on their session + instance id, so two instances of
/// the same plugin name never collide. Global plugins key on their name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PluginDataKey {
    /// Data scoped to a specific attached plugin instance in a session.
    Attached(SessionId, PluginInstanceId),
    /// Data scoped to a global plugin (not bound to any session/instance).
    Global(String),
}

/// Thread-safe shared data store for plugins.
///
/// Wrapped in `Arc` so it can be cloned cheaply and shared between
/// the sync (`SyncPlugins`) and async (`AsyncPluginHandle`) halves
/// of the plugin system.
#[derive(Clone)]
pub struct PluginData(Arc<DashMap<PluginDataKey, serde_json::Value>>);

impl PluginData {
    /// Create a new empty plugin data store.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Get a snapshot of an attached instance's data for a specific session.
    ///
    /// Returns a cloned `serde_json::Value` — the snapshot is taken
    /// at the moment of the call and won't reflect subsequent writes.
    #[must_use]
    pub fn get_for_session(
        &self,
        session_id: &SessionId,
        instance_id: &PluginInstanceId,
    ) -> Option<serde_json::Value> {
        self.0
            .get(&PluginDataKey::Attached(session_id.clone(), instance_id.clone()))
            .map(|v| v.clone())
    }

    /// Set an attached instance's data for a specific session. Replaces any previous value.
    pub fn set_for_session(
        &self,
        session_id: &SessionId,
        instance_id: &PluginInstanceId,
        value: serde_json::Value,
    ) {
        self.0.insert(
            PluginDataKey::Attached(session_id.clone(), instance_id.clone()),
            value,
        );
    }

    /// Shallow-merge a partial object into an attached instance's data for a specific session.
    ///
    /// Top-level keys in `partial` overwrite the same keys in the stored
    /// value; other top-level keys are untouched. The stored value is
    /// treated as `{}` when no data exists yet, or when the stored value
    /// is not an object (a stray scalar is replaced wholesale). Nested
    /// objects in `partial` replace — are not deep-merged into — the
    /// corresponding nested objects in the stored value.
    pub fn merge_for_session(
        &self,
        session_id: &SessionId,
        instance_id: &PluginInstanceId,
        partial: serde_json::Value,
    ) {
        let key = PluginDataKey::Attached(session_id.clone(), instance_id.clone());
        let merged = Self::merge_existing(&self.0, &key, partial);
        self.0.insert(key, merged);
    }

    /// Get a snapshot of a global plugin's data.
    #[must_use]
    pub fn get(&self, plugin_name: &str) -> Option<serde_json::Value> {
        self.0
            .get(&PluginDataKey::Global(plugin_name.to_owned()))
            .map(|v| v.clone())
    }

    /// Set a global plugin's data. Replaces any previous value.
    pub fn set(&self, plugin_name: &str, value: serde_json::Value) {
        self.0
            .insert(PluginDataKey::Global(plugin_name.to_owned()), value);
    }

    /// Shallow-merge into a global plugin's data.
    pub fn merge(&self, plugin_name: &str, partial: serde_json::Value) {
        let key = PluginDataKey::Global(plugin_name.to_owned());
        let merged = Self::merge_existing(&self.0, &key, partial);
        self.0.insert(key, merged);
    }

    /// Shared shallow-merge helper for both attached and global scopes.
    fn merge_existing(
        map: &DashMap<PluginDataKey, serde_json::Value>,
        key: &PluginDataKey,
        partial: serde_json::Value,
    ) -> serde_json::Value {
        use serde_json::Value;
        map.get(key)
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
            .unwrap_or(partial)
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

    fn session(name: &str) -> SessionId {
        SessionId::from(name.to_owned())
    }

    #[test]
    fn get_returns_none_for_missing_global_plugin() {
        let data = PluginData::new();
        assert!(data.get("nonexistent").is_none());
    }

    #[test]
    fn set_then_get_roundtrips_global() {
        let data = PluginData::new();
        data.set("alpha", serde_json::json!({ "verdict": "pass" }));
        let result = data.get("alpha").expect("should exist");
        assert_eq!(result["verdict"], "pass");
    }

    #[test]
    fn set_overwrites_previous_value_global() {
        let data = PluginData::new();
        data.set("beta", serde_json::json!("first"));
        data.set("beta", serde_json::json!("second"));
        assert_eq!(data.get("beta"), Some(serde_json::json!("second")));
    }

    #[test]
    fn global_plugins_have_isolated_data() {
        let data = PluginData::new();
        data.set("alpha", serde_json::json!(1));
        data.set("beta", serde_json::json!(2));
        assert_eq!(data.get("alpha"), Some(serde_json::json!(1)));
        assert_eq!(data.get("beta"), Some(serde_json::json!(2)));
    }

    #[test]
    fn clone_shares_underlying_storage() {
        let data = PluginData::new();
        let cloned = data.clone();
        data.set("shared", serde_json::json!("hello"));
        assert_eq!(cloned.get("shared"), Some(serde_json::json!("hello")));
    }

    #[test]
    fn merge_overwrites_top_level_keys_and_keeps_others_global() {
        // Given a global plugin with two fields stored.
        let data = PluginData::new();
        data.set("alpha", serde_json::json!({"status": "idle", "count": 3}));

        // When shallow-merging one top-level key.
        data.merge("alpha", serde_json::json!({"status": "enriching"}));

        // Then the merged key updates and the untouched key survives.
        let result = data.get("alpha").expect("should exist");
        assert_eq!(result["status"], "enriching");
        assert_eq!(result["count"], 3);
    }

    #[test]
    fn two_sessions_dont_collide() {
        // Given a plugin instance in two different sessions.
        let data = PluginData::new();
        let s1 = session("session-1");
        let s2 = session("session-2");
        let inst = PluginInstanceId::new();

        data.set_for_session(&s1, &inst, serde_json::json!({ "verdict": "pass" }));
        data.set_for_session(&s2, &inst, serde_json::json!({ "verdict": "fail" }));

        // Then each session reads its own data.
        assert_eq!(
            data.get_for_session(&s1, &inst),
            Some(serde_json::json!({ "verdict": "pass" }))
        );
        assert_eq!(
            data.get_for_session(&s2, &inst),
            Some(serde_json::json!({ "verdict": "fail" }))
        );
    }

    #[test]
    fn two_instances_in_same_session_have_isolated_data() {
        // Given two instances of the same plugin name in one session.
        let data = PluginData::new();
        let s1 = session("session-1");
        let inst_a = PluginInstanceId::new();
        let inst_b = PluginInstanceId::new();

        data.set_for_session(&s1, &inst_a, serde_json::json!({ "verdict": "pass" }));
        data.set_for_session(&s1, &inst_b, serde_json::json!({ "verdict": "fail" }));

        // Then each instance reads only its own data.
        assert_eq!(
            data.get_for_session(&s1, &inst_a),
            Some(serde_json::json!({ "verdict": "pass" }))
        );
        assert_eq!(
            data.get_for_session(&s1, &inst_b),
            Some(serde_json::json!({ "verdict": "fail" }))
        );
    }

    #[test]
    fn merge_for_session_scopes_correctly() {
        let data = PluginData::new();
        let s1 = session("session-1");
        let s2 = session("session-2");
        let inst_a = PluginInstanceId::new();
        let inst_b = PluginInstanceId::new();

        data.set_for_session(&s1, &inst_a, serde_json::json!({ "count": 1 }));
        data.set_for_session(&s2, &inst_b, serde_json::json!({ "count": 1 }));

        data.merge_for_session(&s1, &inst_a, serde_json::json!({ "extra": "data" }));

        let s1_data = data.get_for_session(&s1, &inst_a).expect("s1");
        let s2_data = data.get_for_session(&s2, &inst_b).expect("s2");

        assert_eq!(s1_data["count"], 1);
        assert_eq!(s1_data["extra"], "data");
        assert_eq!(s2_data["count"], 1);
        assert!(s2_data.get("extra").is_none());
    }
}
