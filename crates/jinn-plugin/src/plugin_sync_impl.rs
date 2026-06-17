//! PluginSyncCall trait implementation for PluginSyncHandle.

use crate::SessionRegistryId;
use error_stack::Report;
use jinn_domain::feat::plugin_dispatch::{PluginSyncCall, PluginSyncCallError};
use serde_json::Value;

use super::sync_handle::PluginSyncHandle;

impl PluginSyncCall for PluginSyncHandle {
    fn call_hooks_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginSyncCallError>> {
        self.call_hooks_json_impl(hook, ctx)
            .map_err(|report| report.change_context(PluginSyncCallError))
    }

    fn call_hooks_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginSyncCallError>> {
        self.call_hooks_for_session_json_impl(session, hook, ctx)
            .map_err(|report| report.change_context(PluginSyncCallError))
    }

    fn name(&self) -> &'static str {
        "PluginSyncHandle"
    }
}
