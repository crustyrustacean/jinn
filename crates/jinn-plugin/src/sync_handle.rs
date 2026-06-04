//! Sync-side plugin handle — blocking hook calls from actor threads.
//!
//! [`PluginSyncHandle`] is `Send + Sync + Clone`. It sends jobs through the
//! same kanal channel as [`AsyncPluginHandle`], then blocks on a kanal
//! unbounded response channel.
//!
//! **Do not call from the render thread** — use
//! [`SyncPlugins`](crate::SyncPlugins) instead.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::async_handle::PluginJob;

/// Handle for calling plugin hooks synchronously from actor threads.
///
/// `Send + Sync + Clone`. Cheap to clone.
#[derive(Clone)]
pub struct PluginSyncHandle {
    /// Shared channel sender (same channel as AsyncPluginHandle).
    pub(crate) tx: kanal::Sender<PluginJob>,
}

impl PluginSyncHandle {
    /// Call all hooks, collecting return values. Blocks the calling thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    pub fn call_hooks<T: Serialize, R: DeserializeOwned>(
        &self,
        hook: &str,
        ctx: &T,
    ) -> Result<Vec<R>, String> {
        let ctx_json = serde_json::to_value(ctx).map_err(|e| format!("serialize ctx: {e}"))?;
        self.call_hooks_json(hook, &ctx_json)
            .map(|results| {
                results
                    .into_iter()
                    .map(|v| serde_json::from_value(v).map_err(|e| format!("deserialize: {e}")))
                    .collect()
            })
            .and_then(|r| r)
    }

    /// Call hooks with raw JSON context.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    pub fn call_hooks_json(
        &self,
        hook: &str,
        ctx_json: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, String> {
        let (respond_tx, respond_rx) =
            kanal::unbounded::<Result<Vec<serde_json::Value>, String>>();
        self.tx
            .send(PluginJob::SyncCollect {
                hook: hook.to_owned(),
                ctx_json: ctx_json.clone(),
                respond_to: respond_tx,
            })
            .map_err(|e| format!("send to plugin thread: {e}"))?;
        respond_rx
            .recv()
            .map_err(|_| "plugin thread died".to_owned())?
    }
}
