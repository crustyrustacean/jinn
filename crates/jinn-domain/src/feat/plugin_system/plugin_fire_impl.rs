//! PluginFire trait implementation for AsyncPluginHandle.
//!
//! This impl lives in `jinn-plugin` (where `AsyncPluginHandle` is defined)
//! to satisfy the orphan rule. The trait `PluginFire` is defined in
//! `jinn-domain`, which `jinn-plugin` depends on.

use crate::feat::plugin_dispatch::{PluginFire, PluginFireError};
use crate::feat::plugin_system::SessionRegistryId;
use error_stack::Report;
use serde_json::Value;

use super::async_handle::AsyncPluginHandle;

#[async_trait::async_trait]
impl PluginFire for AsyncPluginHandle {
    async fn fire_async_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<(), Report<PluginFireError>> {
        self.fire_async(hook, ctx)
            .await
            .map_err(|report| report.change_context(PluginFireError))
    }

    async fn fire_async_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
        enabled_plugins: Vec<String>,
    ) -> Result<(), Report<PluginFireError>> {
        self.fire_async_for_session(Some(session), hook, ctx, enabled_plugins)
            .await
            .map_err(|report| report.change_context(PluginFireError))
    }

    async fn fire_async_collect_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginFireError>> {
        self.fire_async_collect(hook, ctx)
            .await
            .map_err(|report| report.change_context(PluginFireError))
    }

    async fn fire_async_collect_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginFireError>> {
        self.fire_async_collect_for_session(Some(session), hook, ctx)
            .await
            .map_err(|report| report.change_context(PluginFireError))
    }

    async fn execute_plugin_tool(
        &self,
        target: Option<SessionRegistryId>,
        session_id: &crate::SessionId,
        parent_session_id: Option<&crate::SessionId>,
        plugin_name: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<String, Report<PluginFireError>> {
        self.execute_tool(
            target,
            session_id.clone(),
            parent_session_id.cloned(),
            plugin_name,
            tool_name,
            arguments,
        )
        .await
        .map_err(|report| report.change_context(PluginFireError))
    }

    fn name(&self) -> &'static str {
        "AsyncPluginHandle"
    }
}
