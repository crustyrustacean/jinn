//! PluginFire trait implementation for AsyncPluginHandle.
//!
//! This impl lives in `jinn-plugin` (where `AsyncPluginHandle` is defined)
//! to satisfy the orphan rule. The trait `PluginFire` is defined in
//! `jinn-domain`, which `jinn-plugin` depends on.

use error_stack::Report;
use jinn_domain::feat::workflow::{PluginFire, PluginFireError};
use serde_json::Value;

use crate::AsyncPluginHandle;

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

    async fn fire_async_collect_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginFireError>> {
        self.fire_async_collect(hook, ctx)
            .await
            .map_err(|report| report.change_context(PluginFireError))
    }

    fn name(&self) -> &'static str {
        "AsyncPluginHandle"
    }
}
