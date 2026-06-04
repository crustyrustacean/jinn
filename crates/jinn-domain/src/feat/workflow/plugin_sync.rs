//! Plugin sync call trait — abstraction for blocking sync hook calls.
//!
//! The domain layer can't depend on `jinn-plugin` (circular dependency), so
//! this trait provides the minimal interface for sync hook calling.
//! `jinn-plugin` provides the concrete implementation for `PluginSyncHandle`.

use serde_json::Value;

/// Call plugin hooks synchronously, collecting return values.
///
/// Implemented by `jinn_plugin::PluginSyncHandle`.
///
/// Blocks the calling thread until all hooks complete. Use from actor
/// message handlers that need plugin return values before proceeding.
///
/// **Do not use from async contexts** — use
/// [`PluginFire::fire_async_collect_json`](super::PluginFire::fire_async_collect_json)
/// instead.
pub trait PluginSyncCall: Send + Sync {
    /// Call all hooks for the given name, collecting non-nil return values.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    fn call_hooks_json(&self, hook: &str, ctx: &Value) -> Result<Vec<Value>, String>;
}
