//! Async-side plugin handle — sends hook fire requests to the background thread.
//!
//! [`AsyncPluginHandle`] is `Send + Sync + Clone`. It holds only a channel
//! sender and the shared plugin data store. The actual Lua execution happens
//! on a dedicated background thread (see `async_thread.rs`).

use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::oneshot;
use wherror::Error;

use crate::PluginData;

/// Error type for plugin system failures.
///
/// Colocated with [`AsyncPluginHandle`] because it is the primary consumer
/// and constructs the channel + oneshot plumbing whose failure modes this
/// type describes. Specific failure reasons are attached to the `Report`
/// via `.attach("...")` calls.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PluginError;

/// Internal message sent to the background thread — both async and sync jobs.
///
/// All three variants respond through [`tokio::sync::oneshot`]. Async callers
/// `.await` the receiver; sync callers call `.blocking_recv()`.
pub(crate) enum PluginJob {
    /// Fire all hooks, discard return values (async).
    Fire {
        /// Hook name to fire.
        hook: String,
        /// Serialized hook ctx.
        ctx_json: serde_json::Value,
        /// Oneshot responder.
        respond_to: oneshot::Sender<Result<(), Report<PluginError>>>,
    },
    /// Fire all hooks, collect return values (async).
    Collect {
        /// Hook name to fire.
        hook: String,
        /// Serialized hook ctx.
        ctx_json: serde_json::Value,
        /// Oneshot responder.
        respond_to: oneshot::Sender<Result<Vec<serde_json::Value>, Report<PluginError>>>,
    },
    /// Fire all hooks, collect return values (sync, blocking caller).
    SyncCollect {
        /// Hook name to fire.
        hook: String,
        /// Serialized hook ctx.
        ctx_json: serde_json::Value,
        /// Oneshot responder (sync caller will `blocking_recv()`).
        respond_to: oneshot::Sender<Result<Vec<serde_json::Value>, Report<PluginError>>>,
    },
}

/// Handle for firing async hooks. Send, Sync, and Clone.
///
/// Cloning is cheap — just clones a channel sender and an `Arc<DashMap>`.
#[derive(Clone)]
pub struct AsyncPluginHandle {
    pub(crate) tx: kanal::AsyncSender<PluginJob>,
    pub(crate) plugin_data: PluginData,
}

impl AsyncPluginHandle {
    /// Fire an async hook on the background thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead, the call times out,
    /// or a hook errors.
    pub async fn fire_async<T: Serialize>(
        &self,
        hook: &str,
        ctx: &T,
    ) -> Result<(), Report<PluginError>> {
        let ctx_json = serde_json::to_value(ctx)
            .change_context(PluginError)
            .attach("failed to serialize hook ctx")?;
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(PluginJob::Fire {
                hook: hook.to_owned(),
                ctx_json,
                respond_to,
            })
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("failed to send Fire job to plugin thread")
            .attach(hook.to_owned())?;
        tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("timed out waiting for plugin thread response (30s)")
            .attach(hook.to_owned())?
            .map_err(|_e| Report::new(PluginError))
            .attach("plugin thread dropped oneshot responder")
            .attach(hook.to_owned())??;
        Ok(())
    }

    /// Fire an async hook, collecting return values.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead, the call times out,
    /// or a hook errors.
    pub async fn fire_async_collect<T: Serialize, R: DeserializeOwned>(
        &self,
        hook: &str,
        ctx: &T,
    ) -> Result<Vec<R>, Report<PluginError>> {
        let ctx_json = serde_json::to_value(ctx)
            .change_context(PluginError)
            .attach("failed to serialize hook ctx")?;
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(PluginJob::Collect {
                hook: hook.to_owned(),
                ctx_json,
                respond_to,
            })
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("failed to send Collect job to plugin thread")
            .attach(hook.to_owned())?;
        let results: Vec<serde_json::Value> =
            tokio::time::timeout(std::time::Duration::from_secs(30), rx)
                .await
                .map_err(|_e| Report::new(PluginError))
                .attach("timed out waiting for plugin thread response (30s)")
                .attach(hook.to_owned())?
                .map_err(|_e| Report::new(PluginError))
                .attach("plugin thread dropped oneshot responder")
                .attach(hook.to_owned())??;
        results
            .into_iter()
            .map(|v| {
                serde_json::from_value(v)
                    .change_context(PluginError)
                    .attach("failed to deserialize hook return value")
            })
            .collect()
    }

    /// Get a snapshot of a plugin's data.
    #[must_use]
    pub fn get_plugin_data(&self, plugin_name: &str) -> Option<serde_json::Value> {
        self.plugin_data.get(plugin_name)
    }
}
