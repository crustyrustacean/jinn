//! PluginSyncCall trait implementation for PluginSyncHandle.
//!
//! This impl lives in `jinn-plugin` (where `PluginSyncHandle` is defined)
//! to satisfy the orphan rule. The trait `PluginSyncCall` is defined in
//! `jinn-domain`, which `jinn-plugin` depends on.

use error_stack::Report;
use jinn_domain::feat::workflow::{PluginSyncCall, PluginSyncCallError};
use serde_json::Value;

use crate::PluginSyncHandle;

impl PluginSyncCall for PluginSyncHandle {
    fn call_hooks_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginSyncCallError>> {
        self.call_hooks_json(hook, ctx)
            .map_err(|report| report.change_context(PluginSyncCallError))
    }

    fn name(&self) -> &'static str {
        "PluginSyncHandle"
    }
}
