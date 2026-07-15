//! Sync-side handle for blocking hook calls from actor threads.
//!
//! [`SyncWasmHandle`] is `Send + Sync + Clone`. It sends jobs through the same
//! channel as [`crate::AsyncWasmHandle`], then blocks on a
//! `tokio::sync::oneshot` receiver via `blocking_recv()`.
//!
//! **Do not call from the render thread** — sync render hooks (badges,
//! keybind-trigger) run on the render-thread-local sync store set directly
//! via [`crate::PluginSyncHooks`], not through this handle. This handle is for
//! *actor threads* that need to block on plugin return values.

use error_stack::{Report, ResultExt};
use serde_json::Value;
use tokio::sync::oneshot;

use jinn_core_types::SessionRegistryId;

use crate::async_thread::{SyncThreadSender, WasmJob};

use jinn_domain::feat::plugin_dispatch::PluginSyncCallError as DomainPluginSyncCallError;




/// Handle for calling plugin hooks synchronously from actor threads.
///
/// `Send + Sync + Clone`. Cheap to clone. Sends a `SyncCollect` job through
/// the background thread's channel and blocks the caller via
/// `blocking_recv()`.
#[derive(Clone)]
pub struct SyncWasmHandle {
    tx: SyncThreadSender,
}

impl std::fmt::Debug for SyncWasmHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncWasmHandle").finish_non_exhaustive()
    }
}

impl SyncWasmHandle {
    /// Construct from the sync channel sender.
    #[must_use]
    pub(crate) fn new(tx: SyncThreadSender) -> Self {
        Self { tx }
    }

    /// Call all global hooks for a name, collecting return values. Blocks.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead or a hook traps.
    pub fn call_hooks_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<DomainPluginSyncCallError>> {
        self.call_hooks_for_session_json(None, hook, ctx)
    }

    /// Call hooks for a session (globals + attached), collecting returns. Blocks.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead or a hook traps.
    pub fn call_hooks_for_session_json(
        &self,
        session: impl Into<Option<SessionRegistryId>>,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<DomainPluginSyncCallError>> {
        let (respond_tx, respond_rx) = oneshot::channel();
        self.tx
            .send(WasmJob::SyncCollect {
                hook: hook.to_owned(),
                ctx_json: ctx.clone(),
                respond_to: respond_tx,
                target_session: session.into(),
            })
            .map_err(|_e| Report::new(DomainPluginSyncCallError))
            .attach("failed to send SyncCollect job to wasm thread")
            .attach(hook.to_owned())?;
        respond_rx
            .blocking_recv()
            .map_err(|_e| Report::new(DomainPluginSyncCallError))
            .attach("wasm thread dropped oneshot responder")
            .attach(hook.to_owned())?
            .change_context(DomainPluginSyncCallError)
            .attach("sync hook fire errored on the background thread")
}
}

impl jinn_domain::feat::plugin_dispatch::PluginSyncCall for SyncWasmHandle {
    fn call_hooks_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<jinn_domain::feat::plugin_dispatch::PluginSyncCallError>> {
        SyncWasmHandle::call_hooks_json(self, hook, ctx)
    }

    fn call_hooks_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<jinn_domain::feat::plugin_dispatch::PluginSyncCallError>> {
        SyncWasmHandle::call_hooks_for_session_json(self, Some(session), hook, ctx)
    }

    fn name(&self) -> &'static str {
        "SyncWasmHandle"
    }
}
