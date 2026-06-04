//! Plugin fire trait — abstraction for firing async hooks from the domain layer.
//!
//! The domain layer can't depend on `jinn-plugin` (circular dependency), so
//! this trait provides the minimal interface the workflow controller needs.
//! The wiring layer (`src/actor_wiring.rs`) provides the concrete implementation
//! using `AsyncPluginHandle`.

use serde_json::Value;

/// Fire async hooks on the plugin system.
///
/// Implemented by the wiring layer using `jinn_plugin::AsyncPluginHandle`.
/// The workflow controller uses this to fire `on_turn_end` and other hooks.
#[async_trait::async_trait]
pub trait PluginFire: Send + Sync {
    /// Fire an async hook with raw JSON context.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    async fn fire_async_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<(), String>;
}
