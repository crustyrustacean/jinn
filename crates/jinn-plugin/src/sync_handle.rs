//! Sync-side plugin handle — blocking hook calls from actor threads.
//!
//! [`PluginSyncHandle`] is `Send + Sync + Clone`. It sends jobs through a kanal
//! channel to the background plugin thread, then blocks on a kanal bounded
//! channel for the response.
//!
//! Use this when an actor needs plugin return values and can't easily
//! restructure into async. Blocks the calling thread for microseconds
//! (Lua function execution, no I/O).
//!
//! **Do not call from the render thread** — the render thread already has
//! [`SyncPlugins`](crate::SyncPlugins) for direct Lua calls with no hop.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Internal sync job sent from actors to the plugin background thread.
pub(crate) struct SyncJob {
    /// The hook name to fire.
    pub hook: String,
    /// Serialized context data.
    pub ctx_json: serde_json::Value,
    /// Sender for the background thread to send results back.
    pub respond_to: kanal::Sender<Result<Vec<serde_json::Value>, String>>,
}

/// Handle for calling plugin hooks synchronously from actor threads.
///
/// `Send + Sync + Clone`. Cheap to clone — just a channel sender.
///
/// Blocks the calling thread until the background thread processes the job
/// and sends back results. Works from any thread, including tokio worker
/// threads (uses kanal's sync `recv`, not tokio's `blocking_recv`).
///
/// **Do not use from the render thread** — the render thread already has
/// [`SyncPlugins`](crate::SyncPlugins) for direct Lua calls with no hop.
/// For async contexts, use
/// [`AsyncPluginHandle::fire_async_collect`](crate::AsyncPluginHandle::fire_async_collect).
#[derive(Clone)]
pub struct PluginSyncHandle {
    /// Channel sender to the background plugin thread.
    pub(crate) tx: kanal::Sender<SyncJob>,
}

impl PluginSyncHandle {
    /// Call all hooks for the given name, collecting return values.
    ///
    /// Blocks the calling thread until all hooks complete. Plugins that
    /// return `nil` are excluded from results.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead or a hook errors.
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

    /// Call hooks with raw JSON context (no generic serialization).
    ///
    /// Returns collected JSON values.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead or a hook errors.
    pub fn call_hooks_json(
        &self,
        hook: &str,
        ctx_json: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, String> {
        let (respond_tx, respond_rx) = kanal::bounded::<Result<Vec<serde_json::Value>, String>>(1);
        self.tx
            .send(SyncJob {
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
