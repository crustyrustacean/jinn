//! Plugin fire trait — abstraction for firing async hooks from the domain layer.
//!
//! The domain layer can't depend on `jinn-plugin` (circular dependency), so
//! this trait provides the minimal interface for async hook firing.
//! `jinn-plugin` provides the concrete implementation for `AsyncPluginHandle`.

use error_stack::Report;
use serde_json::Value;
use wherror::Error;

/// Error raised by [`PluginFire`] implementations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PluginFireError;

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
    async fn fire_async_json(&self, hook: &str, ctx: &Value)
    -> Result<(), Report<PluginFireError>>;

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
    ) -> Result<Vec<Value>, Report<PluginFireError>>;

    /// Returns the name of this backend for debugging.
    fn name(&self) -> &'static str;
}

use derive_more::Debug;
use std::sync::Arc;

/// Service wrapper for [`PluginFire`].
///
/// Cheap to clone (Arc). Construct once at startup, share via [`crate::Services`].
#[derive(Debug, Clone)]
pub struct PluginFireService {
    #[debug("PluginFire<{}>", self.backend.name())]
    backend: Arc<dyn PluginFire>,
}

impl PluginFireService {
    /// Construct a new service wrapper around a [`PluginFire`] backend.
    #[must_use]
    pub fn new(backend: Arc<dyn PluginFire>) -> Self {
        Self { backend }
    }

    /// Fire an async hook (return values discarded).
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    pub async fn fire_async_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<(), Report<PluginFireError>> {
        self.backend.fire_async_json(hook, ctx).await
    }

    /// Fire an async hook, collecting return values from all plugins.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    pub async fn fire_async_collect_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginFireError>> {
        self.backend.fire_async_collect_json(hook, ctx).await
    }

    /// Returns the backend name for debugging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.backend.name()
    }
}
