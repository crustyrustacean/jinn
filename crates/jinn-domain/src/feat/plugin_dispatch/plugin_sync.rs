//! Plugin sync call trait — abstraction for blocking sync hook calls.
//!
//! The domain layer can't depend on `jinn-wasm-host` (circular dependency), so
//! this trait provides the minimal interface for sync hook calling.
//! `jinn-wasm-host` provides the concrete implementation for `PluginSyncHandle`.

use error_stack::Report;
use serde_json::Value;
use wherror::Error;

use jinn_core_types::SessionRegistryId;

/// Error raised by [`PluginSyncCall`] implementations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PluginSyncCallError;

/// Call plugin hooks synchronously, collecting return values.
///
/// Implemented by `jinn_wasm_host::SyncWasmHandle`.
///
/// Blocks the calling thread until all hooks complete. Use from actor
/// message handlers that need plugin return values before proceeding.
///
/// **Do not use from async contexts** — use
/// [`PluginFire::fire_async_collect_json`](super::PluginFire::fire_async_collect_json)
/// instead.
pub trait PluginSyncCall: Send + Sync {
    /// Call all global hooks for the given name, collecting non-nil return values.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    fn call_hooks_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginSyncCallError>>;

    /// Call hooks for a session (globals + session's attached), collecting returns.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    fn call_hooks_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginSyncCallError>>;

    /// Returns the name of this backend for debugging.
    fn name(&self) -> &'static str;
}

use derive_more::Debug;
use std::sync::Arc;

/// Service wrapper for [`PluginSyncCall`].
///
/// Cheap to clone (Arc). Construct once at startup, share via [`crate::Services`].
#[derive(Debug, Clone)]
pub struct PluginSyncCallService {
    #[debug("PluginSyncCall<{}>", self.backend.name())]
    backend: Arc<dyn PluginSyncCall>,
}

impl PluginSyncCallService {
    /// Construct a new service wrapper around a [`PluginSyncCall`] backend.
    #[must_use]
    pub fn new(backend: Arc<dyn PluginSyncCall>) -> Self {
        Self { backend }
    }

    /// Call all global hooks for the given name, collecting non-nil return values.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    pub fn call_hooks(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginSyncCallError>> {
        self.backend.call_hooks_json(hook, ctx)
    }

    /// Call hooks for a session (globals + session's attached), collecting returns.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    pub fn call_hooks_for_session(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginSyncCallError>> {
        self.backend.call_hooks_for_session_json(session, hook, ctx)
    }

    /// Returns the backend name for debugging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.backend.name()
    }
}
