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
//! Keyed by `(SessionId, plugin_name)`. Global plugins use
//! [`GLOBAL_SESSION_ID`] as the session key. Each plugin sees only its own
//! data, scoped to the session it was invoked for.

use crate::SessionId;
use dashmap::DashMap;
use std::sync::Arc;

/// Sentinel session ID used for global plugins that don't belong to a session.
///
/// Global plugins share a single entry in the store, keyed by this ID.
pub const GLOBAL_SESSION_ID: &str = "__global__";

/// Thread-safe shared data store for plugins.
///
/// Wrapped in `Arc` so it can be cloned cheaply and shared between
/// the sync (`SyncPlugins`) and async (`AsyncPluginHandle`) halves
/// of the plugin system.
#[derive(Clone)]
pub struct PluginData(Arc<DashMap<(SessionId, String), serde_json::Value>>);

impl PluginData {
    /// Create a new empty plugin data store.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Get a snapshot of a plugin's data for a specific session.
    ///
    /// Returns a cloned `serde_json::Value` — the snapshot is taken
    /// at the moment of the call and won't reflect subsequent writes.
    #[must_use]
    pub fn get_for_session(
        &self,
        session_id: Option<&SessionId>,
        plugin_name: &str,
    ) -> Option<serde_json::Value> {
        let key = self.make_key(session_id, plugin_name);
        self.0.get(&key).map(|v| v.clone())
    }

    /// Set a plugin's data for a specific session. Replaces any previous value.
    pub fn set_for_session(
        &self,
        session_id: Option<&SessionId>,
        plugin_name: &str,
        value: serde_json::Value,
    ) {
        let key = self.make_key(session_id, plugin_name);
        self.0.insert(key, value);
    }

    /// Shallow-merge a partial object into a plugin's data for a specific session.
    ///
    /// Top-level keys in `partial` overwrite the same keys in the stored
    /// value; other top-level keys are untouched. The stored value is
    /// treated as `{}` when no data exists yet, or when the stored value
    /// is not an object (a stray scalar is replaced wholesale). Nested
    /// objects in `partial` replace — are not deep-merged into — the
    /// corresponding nested objects in the stored value.
    pub fn merge_for_session(
        &self,
        session_id: Option<&SessionId>,
        plugin_name: &str,
        partial: serde_json::Value,
    ) {
        use serde_json::Value;

        let key = self.make_key(session_id, plugin_name);
        let merged = self
            .0
            .get(&key)
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
        self.0.insert(key, merged);
    }

    /// Get a snapshot of a global plugin's data.
    #[must_use]
    pub fn get(&self, plugin_name: &str) -> Option<serde_json::Value> {
        self.get_for_session(None, plugin_name)
    }

    /// Set a global plugin's data. Replaces any previous value.
    pub fn set(&self, plugin_name: String, value: serde_json::Value) {
        self.set_for_session(None, &plugin_name, value);
    }

    /// Shallow-merge into a global plugin's data.
    pub fn merge(&self, plugin_name: &str, partial: serde_json::Value) {
        self.merge_for_session(None, plugin_name, partial);
    }

    /// Build the composite key. Uses [`GLOBAL_SESSION_ID`] when no session is provided.
    fn make_key(&self, session_id: Option<&SessionId>, plugin_name: &str) -> (SessionId, String) {
        let sid = session_id
            .cloned()
            .unwrap_or_else(|| SessionId::from(GLOBAL_SESSION_ID.to_owned()));
        (sid, plugin_name.to_owned())
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

    #[test]
    fn two_sessions_dont_collide() {
        // Given a plugin "judge" with data for two different sessions.
        let data = PluginData::new();
        let s1 = session("session-1");
        let s2 = session("session-2");

        data.set_for_session(Some(&s1), "judge", serde_json::json!({ "verdict": "pass" }));
        data.set_for_session(Some(&s2), "judge", serde_json::json!({ "verdict": "fail" }));

        // Then each session reads its own data.
        assert_eq!(
            data.get_for_session(Some(&s1), "judge"),
            Some(serde_json::json!({ "verdict": "pass" }))
        );
        assert_eq!(
            data.get_for_session(Some(&s2), "judge"),
            Some(serde_json::json!({ "verdict": "fail" }))
        );
    }

    #[test]
    fn global_and_session_data_are_separate() {
        let data = PluginData::new();
        let s1 = session("session-1");

        // Global write (no session).
        data.set("judge".to_owned(), serde_json::json!({ "global": true }));
        // Session-scoped write.
        data.set_for_session(Some(&s1), "judge", serde_json::json!({ "global": false }));

        // Global read returns global data.
        assert_eq!(
            data.get("judge"),
            Some(serde_json::json!({ "global": true }))
        );
        // Session read returns session data.
        assert_eq!(
            data.get_for_session(Some(&s1), "judge"),
            Some(serde_json::json!({ "global": false }))
        );
    }

    #[test]
    fn merge_for_session_scopes_correctly() {
        let data = PluginData::new();
        let s1 = session("session-1");
        let s2 = session("session-2");

        data.set_for_session(Some(&s1), "judge", serde_json::json!({ "count": 1 }));
        data.set_for_session(Some(&s2), "judge", serde_json::json!({ "count": 1 }));

        data.merge_for_session(Some(&s1), "judge", serde_json::json!({ "extra": "data" }));

        let s1_data = data.get_for_session(Some(&s1), "judge").expect("s1");
        let s2_data = data.get_for_session(Some(&s2), "judge").expect("s2");

        assert_eq!(s1_data["count"], 1);
        assert_eq!(s1_data["extra"], "data");
        assert_eq!(s2_data["count"], 1);
        assert!(s2_data.get("extra").is_none());
    }
}
