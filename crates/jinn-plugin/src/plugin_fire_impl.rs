//! PluginFire trait implementation for AsyncPluginHandle.
//!
//! This impl lives in `jinn-plugin` (where `AsyncPluginHandle` is defined)
//! to satisfy the orphan rule. The trait `PluginFire` is defined in
//! `jinn-domain`, which `jinn-plugin` depends on.

use jinn_domain::feat::workflow::PluginFire;
use serde_json::Value;

use crate::AsyncPluginHandle;

#[async_trait::async_trait]
impl PluginFire for AsyncPluginHandle {
    async fn fire_async_json(&self, hook: &str, ctx: &Value) -> Result<(), String> {
        self.fire_async(hook, ctx).await
    }
}
