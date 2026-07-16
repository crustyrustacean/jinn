//! Plugin fire trait — abstraction for firing async hooks from the domain layer.
//!
//! The domain layer can't depend on `jinn-wasm-host` (circular dependency), so
//! this trait provides the minimal interface for async hook firing.
//! `jinn-wasm-host` provides the concrete implementation for `AsyncWasmHandle`.

use error_stack::Report;
use serde_json::Value;
use wherror::Error;

use jinn_core_types::{PluginInstanceId, SessionRegistryId};

use super::plugin_ctx::HookCtx;

/// Error raised by [`PluginFire`] implementations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PluginFireError;

/// Fire async hooks on the plugin system.
///
/// Implemented by `jinn_wasm_host::AsyncWasmHandle`. The hook *name* is a
/// `&str` because plugin-defined triggers (`on_enrich`, …) are resolved by
/// string at runtime; the *payload* is a typed [`HookCtx`].
#[async_trait::async_trait]
pub trait PluginFire: Send + Sync {
    /// Fire an async hook with a typed context (global plugins only).
    ///
    /// All global hooks for the given name run on the background thread.
    /// Return values are discarded.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    async fn fire_async(&self, hook: &str, ctx: &HookCtx)
    -> Result<(), Report<PluginFireError>>;

    /// Fire an async hook with a typed context (global + session plugins).
    ///
    /// Additionally fires hooks from the named session's attached plugins
    /// (filtered by `enabled_instances` if provided) after the global set.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    async fn fire_async_for_session(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &HookCtx,
        enabled_instances: Option<Vec<PluginInstanceId>>,
    ) -> Result<(), Report<PluginFireError>>;

    /// Execute a plugin-defined tool handler on the background thread.
    ///
    /// Routes to the correct instance (global or per-session), finds the tool
    /// handler, calls it with `arguments`, and returns the result string.
    ///
    /// `arguments` stays `&Value` because tool-call arguments are the LLM's
    /// JSON-schema-shaped output — genuinely schema-shaped JSON, not a
    /// host-known shape.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable, the tool handler
    /// is not found, or the handler itself errors.
    async fn execute_plugin_tool(
        &self,
        target: Option<SessionRegistryId>,
        session_id: &crate::protocol::SessionId,
        parent_session_id: Option<&crate::protocol::SessionId>,
        plugin_name: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<String, Report<PluginFireError>>;

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

    /// Fire an async hook (global plugins only, return values discarded).
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    pub async fn fire_async(
        &self,
        hook: &str,
        ctx: &HookCtx,
    ) -> Result<(), Report<PluginFireError>> {
        self.backend.fire_async(hook, ctx).await
    }

    /// Fire an async hook (global + session plugins, return values discarded).
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    pub async fn fire_async_for_session(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &HookCtx,
        enabled_instances: Option<Vec<PluginInstanceId>>,
    ) -> Result<(), Report<PluginFireError>> {
        self.backend
            .fire_async_for_session(session, hook, ctx, enabled_instances)
            .await
    }

    /// Execute a plugin-defined tool handler on the background thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable, the tool handler
    /// is not found, or the handler itself errors.
    pub async fn execute_plugin_tool(
        &self,
        target: Option<SessionRegistryId>,
        session_id: &crate::protocol::SessionId,
        parent_session_id: Option<&crate::protocol::SessionId>,
        plugin_name: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<String, Report<PluginFireError>> {
        self.backend
            .execute_plugin_tool(
                target,
                session_id,
                parent_session_id,
                plugin_name,
                tool_name,
                arguments,
            )
            .await
    }

    /// Returns the backend name for debugging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.backend.name()
    }
}
