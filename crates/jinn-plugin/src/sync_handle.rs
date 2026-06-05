//! Sync-side plugin handle — blocking hook calls from actor threads.
//!
//! [`PluginSyncHandle`] is `Send + Sync + Clone`. It sends jobs through the
//! same kanal channel as [`AsyncPluginHandle`], then blocks on a
//! `tokio::sync::oneshot` receiver via `blocking_recv()`.
//!
//! **Do not call from the render thread** — use
//! [`SyncPlugins`](crate::SyncPlugins) instead.

use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::oneshot;

use crate::async_handle::{PluginError, PluginJob};

/// Handle for calling plugin hooks synchronously from actor threads.
///
/// `Send + Sync + Clone`. Cheap to clone.
#[derive(Clone)]
pub struct PluginSyncHandle {
    /// Sync channel sender sharing the same channel as `AsyncPluginHandle`.
    /// Derived via `clone_sync()` so the async sender stays valid.
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
    ) -> Result<Vec<R>, Report<PluginError>> {
        let ctx_json = serde_json::to_value(ctx)
            .change_context(PluginError)
            .attach("failed to serialize hook ctx")?;
        let results = self.call_hooks_json(hook, &ctx_json)?;
        results
            .into_iter()
            .map(|v| {
                serde_json::from_value(v)
                    .change_context(PluginError)
                    .attach("failed to deserialize hook return value")
            })
            .collect()
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
    ) -> Result<Vec<serde_json::Value>, Report<PluginError>> {
        let (respond_tx, respond_rx) = oneshot::channel();
        self.tx
            .send(PluginJob::SyncCollect {
                hook: hook.to_owned(),
                ctx_json: ctx_json.clone(),
                respond_to: respond_tx,
            })
            .map_err(|_e| Report::new(PluginError))
            .attach("failed to send SyncCollect job to plugin thread")
            .attach(hook.to_owned())?;
        respond_rx
            .blocking_recv()
            .map_err(|_e| Report::new(PluginError))
            .attach("plugin thread dropped oneshot responder")
            .attach(hook.to_owned())?
    }
}
