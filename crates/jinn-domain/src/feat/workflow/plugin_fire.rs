//! Plugin fire trait — abstraction for firing async hooks from the domain layer.
//!
//! The domain layer can't depend on `jinn-plugin` (circular dependency), so
//! this trait provides the minimal interface for async hook firing.
//! `jinn-plugin` provides the concrete implementation for `AsyncPluginHandle`.

use serde_json::Value;

/// Fire async hooks on the plugin system.
///
/// Implemented by `jinn_plugin::AsyncPluginHandle`.
#[async_trait::async_trait]
pub trait PluginFire: Send + Sync {
    /// Fire an async hook with raw JSON context.
    ///
    /// All hooks for the given name run on the background thread.
    /// Return values are discarded.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    async fn fire_async_json(&self, hook: &str, ctx: &Value) -> Result<(), String>;

    /// Fire an async hook, collecting return values from all plugins.
    ///
    /// Like [`fire_async_json`](Self::fire_async_json), but each plugin's
    /// non-nil return value is collected into the returned `Vec`.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    async fn fire_async_collect_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, String>;
}
