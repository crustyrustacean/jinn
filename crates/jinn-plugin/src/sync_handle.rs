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
use crate::session_registry::SessionRegistryId;

/// Handle for calling plugin hooks synchronously from actor threads.
///
/// `Send + Sync + Clone`. Cheap to clone.
#[derive(Clone)]
pub struct PluginSyncHandle {
    /// Sync channel sender sharing the same channel as `AsyncPluginHandle`.
    /// Derived via `clone_sync()` so the async sender stays valid.
    tx: kanal::Sender<PluginJob>,
}

impl PluginSyncHandle {
    /// Construct a sync plugin handle from its sync channel sender.
    ///
    /// Called by [`crate::PluginSystem::build`] with a `clone_sync()`
    /// sender so the async handle's sender remains valid.
    pub(crate) fn new(tx: kanal::Sender<PluginJob>) -> Self {
        Self { tx }
    }
}

impl PluginSyncHandle {
    /// Call all hooks, collecting return values. Blocks the calling thread.
    ///
    /// Equivalent to `call_hooks_for_session(None, hook, ctx)`.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    pub fn call_hooks<T: Serialize, R: DeserializeOwned>(
        &self,
        hook: &str,
        ctx: &T,
    ) -> Result<Vec<R>, Report<PluginError>> {
        self.call_hooks_for_session(None, hook, ctx)
    }

    /// Call hooks for a specific session's attached plugins + globals.
    /// Blocks the calling thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    pub fn call_hooks_for_session<T: Serialize, R: DeserializeOwned>(
        &self,
        target_session: Option<SessionRegistryId>,
        hook: &str,
        ctx: &T,
    ) -> Result<Vec<R>, Report<PluginError>> {
        let ctx_json = serde_json::to_value(ctx)
            .change_context(PluginError)
            .attach("failed to serialize hook ctx")?;
        let results = self.call_hooks_json_for_session(target_session, hook, &ctx_json)?;
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
    pub fn call_hooks_json_impl(
        &self,
        hook: &str,
        ctx_json: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, Report<PluginError>> {
        self.call_hooks_json_for_session(None, hook, ctx_json)
    }

    /// Call hooks with raw JSON context, optionally scoped to a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    pub fn call_hooks_json_for_session(
        &self,
        target_session: Option<SessionRegistryId>,
        hook: &str,
        ctx_json: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, Report<PluginError>> {
        let (respond_tx, respond_rx) = oneshot::channel();
        self.tx
            .send(PluginJob::SyncCollect {
                hook: hook.to_owned(),
                ctx_json: ctx_json.clone(),
                respond_to: respond_tx,
                target_session,
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

    /// Call hooks with raw JSON context for a specific session (trait impl).
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin thread is dead or a hook errors.
    pub fn call_hooks_for_session_json_impl(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx_json: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, Report<PluginError>> {
        self.call_hooks_json_for_session(Some(session), hook, ctx_json)
    }
}
