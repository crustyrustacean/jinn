//! Async-side plugin handle — sends hook fire requests to the background thread.
//!
//! [`AsyncPluginHandle`] is `Send + Sync + Clone`. It holds only a channel
//! sender and the shared plugin data store. The actual Lua execution happens
//! on a dedicated background thread (see `async_thread.rs`).

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::oneshot;

use crate::PluginData;

/// Internal message sent to the background thread — both async and sync jobs.
pub(crate) enum PluginJob {
    /// Fire all hooks, discard return values (async).
    Fire {
        hook: String,
        ctx_json: serde_json::Value,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    /// Fire all hooks, collect return values (async).
    Collect {
        hook: String,
        ctx_json: serde_json::Value,
        respond_to: oneshot::Sender<Result<Vec<serde_json::Value>, String>>,
    },
    /// Fire all hooks, collect return values (sync, blocking caller).
    SyncCollect {
        hook: String,
        ctx_json: serde_json::Value,
        respond_to: kanal::Sender<Result<Vec<serde_json::Value>, String>>,
    },
}

/// Handle for firing async hooks. Send, Sync, and Clone.
///
/// Cloning is cheap — just clones a channel sender and an `Arc<DashMap>`.
#[derive(Clone)]
pub struct AsyncPluginHandle {
    /// Channel sender to the background thread.
    pub(crate) tx: kanal::Sender<PluginJob>,
    /// Shared plugin data store.
    pub(crate) plugin_data: PluginData,
}

impl AsyncPluginHandle {
    /// Fire an async hook on the background thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead or a hook errors.
    pub async fn fire_async<T: Serialize>(&self, hook: &str, ctx: &T) -> Result<(), String> {
        let ctx_json = serde_json::to_value(ctx).map_err(|e| format!("serialize ctx: {e}"))?;
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(PluginJob::Fire {
                hook: hook.to_owned(),
                ctx_json,
                respond_to,
            })
            .map_err(|e| format!("send to plugin thread: {e}"))?;
        tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| "plugin hook timed out after 30s".to_owned())
            .and_then(|res| res.map_err(|_e| "plugin thread died".to_owned()))?
    }

    /// Fire an async hook, collecting return values.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead or a hook errors.
    pub async fn fire_async_collect<T: Serialize, R: DeserializeOwned>(
        &self,
        hook: &str,
        ctx: &T,
    ) -> Result<Vec<R>, String> {
        let ctx_json = serde_json::to_value(ctx).map_err(|e| format!("serialize ctx: {e}"))?;
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(PluginJob::Collect {
                hook: hook.to_owned(),
                ctx_json,
                respond_to,
            })
            .map_err(|e| format!("send to plugin thread: {e}"))?;
        let results: Vec<serde_json::Value> = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            rx,
        )
        .await
        .map_err(|_| "plugin hook timed out after 30s".to_owned())
        .and_then(|res| res.map_err(|_e| "plugin thread died".to_owned()))??;
        results
            .into_iter()
            .map(|v| serde_json::from_value(v).map_err(|e| format!("deserialize return: {e}")))
            .collect()
    }

    /// Get a snapshot of a plugin's data.
    #[must_use]
    pub fn get_plugin_data(&self, plugin_name: &str) -> Option<serde_json::Value> {
        self.plugin_data.get(plugin_name)
    }
}
