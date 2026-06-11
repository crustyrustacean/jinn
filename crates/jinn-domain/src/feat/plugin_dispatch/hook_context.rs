//! Hook context wrapper for session-aware plugin data access.
//!
//! Wraps a JSON value passed to sync hooks. Provides session-aware data access
//! via the [`ProvidesSessionId`] trait. The plugin system uses the session ID
//! to scope [`PluginData`] lookups to the correct session.

use crate::protocol::SessionId;

/// Wrapper around a JSON value passed to sync hooks.
///
/// Provides session-aware data access via the [`ProvidesSessionId`] trait.
/// Call sites construct this from a `serde_json::Value` via `.into()`.
/// The plugin system calls `.session_id()` to scope plugin_data lookups.
pub struct HookContext {
    inner: serde_json::Value,
}

impl HookContext {
    /// Return a reference to the inner JSON value.
    pub fn value(&self) -> &serde_json::Value {
        &self.inner
    }

    /// Consume the wrapper and return the inner JSON value.
    pub fn to_value(self) -> serde_json::Value {
        self.inner
    }
}

impl From<serde_json::Value> for HookContext {
    fn from(inner: serde_json::Value) -> Self {
        Self { inner }
    }
}

/// Extract the session ID from a hook context.
///
/// Implemented by [`HookContext`] with a default that reads `session_id`
/// from the JSON value. Returns `None` for contexts without a session
/// (e.g. global plugin hooks).
pub trait ProvidesSessionId {
    /// Returns the session ID embedded in this context, if any.
    fn session_id(&self) -> Option<SessionId>;
}

impl ProvidesSessionId for HookContext {
    fn session_id(&self) -> Option<SessionId> {
        self.inner
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| SessionId::from(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_returns_some_when_present() {
        let ctx = HookContext::from(serde_json::json!({ "session_id": "s-123" }));
        let id = ctx.session_id().expect("should have session_id");
        assert_eq!(id, SessionId::from("s-123".to_owned()));
    }

    #[test]
    fn session_id_returns_none_when_absent() {
        let ctx = HookContext::from(serde_json::json!({ "other": "value" }));
        assert!(ctx.session_id().is_none());
    }

    #[test]
    fn session_id_returns_none_for_empty_object() {
        let ctx = HookContext::from(serde_json::json!({}));
        assert!(ctx.session_id().is_none());
    }

    #[test]
    fn to_value_roundtrips() {
        let original = serde_json::json!({ "session_id": "s-1", "key": "val" });
        let ctx = HookContext::from(original.clone());
        assert_eq!(ctx.to_value(), original);
    }

    #[test]
    fn value_returns_reference() {
        let ctx = HookContext::from(serde_json::json!({ "a": 1 }));
        assert_eq!(ctx.value()["a"], 1);
    }
}
