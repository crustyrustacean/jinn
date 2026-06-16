//! Plugin fire trait — abstraction for firing async hooks from the domain layer.
//!
//! The domain layer can't depend on `jinn-plugin` (circular dependency), so
//! this trait provides the minimal interface for async hook firing.
//! `jinn-plugin` provides the concrete implementation for `AsyncPluginHandle`.

use error_stack::Report;
use serde_json::Value;
use wherror::Error;

use crate::feat::plugin_system::SessionRegistryId;

/// Error raised by [`PluginFire`] implementations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PluginFireError;

/// Fire async hooks on the plugin system.
///
/// Implemented by `jinn_plugin::AsyncPluginHandle`.
#[async_trait::async_trait]
pub trait PluginFire: Send + Sync {
    /// Fire an async hook with raw JSON context (global plugins only).
    ///
    /// All global hooks for the given name run on the background thread.
    /// Return values are discarded.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    async fn fire_async_json(&self, hook: &str, ctx: &Value)
    -> Result<(), Report<PluginFireError>>;

    /// Fire an async hook with raw JSON context (global + session plugins).
    ///
    /// Like [`fire_async_json`](Self::fire_async_json), but additionally fires
    /// hooks from the named session's attached plugins after the global set.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    async fn fire_async_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
        enabled_instances: Vec<crate::feat::plugin_system::PluginInstanceId>,
    ) -> Result<(), Report<PluginFireError>>;

    /// Fire an async hook, collecting return values from all global plugins.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    async fn fire_async_collect_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginFireError>>;

    /// Fire an async hook, collecting values from globals + a session's plugins.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    async fn fire_async_collect_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginFireError>>;

    /// Execute a plugin-defined tool handler on the background thread.
    ///
    /// Routes to the correct Lua state (global or per-session),
    /// finds the tool handler, calls it with arguments, returns the result string.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable, the tool handler is not found,
    /// or the handler itself errors.
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
    pub async fn fire_async_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<(), Report<PluginFireError>> {
        self.backend.fire_async_json(hook, ctx).await
    }

    /// Fire an async hook (global + session plugins, return values discarded).
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    pub async fn fire_async_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
        enabled_instances: Vec<crate::feat::plugin_system::PluginInstanceId>,
    ) -> Result<(), Report<PluginFireError>> {
        self.backend
            .fire_async_for_session_json(session, hook, ctx, enabled_instances)
            .await
    }

    /// Fire an async hook, collecting return values from all global plugins.
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

    /// Fire an async hook, collecting values from globals + a session's plugins.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable or a hook errors.
    pub async fn fire_async_collect_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginFireError>> {
        self.backend
            .fire_async_collect_for_session_json(session, hook, ctx)
            .await
    }

    /// Execute a plugin-defined tool handler on the background thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin system is unavailable, the tool handler is not found,
    /// or the handler itself errors.
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
