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
///
/// The optional `target_session` field on fire/collect variants enables
/// per-session plugin execution. When `Some(id)`, the thread additionally
/// consults a per-session Lua state attached via `LoadSessionPlugins`.
/// When `None`, only the shared (global) plugins fire.
pub(crate) enum PluginJob {
    /// Fire all hooks, discard return values (async).
    Fire {
        /// Hook name to fire.
        hook: String,
        /// Serialized hook ctx.
        ctx_json: serde_json::Value,
        /// Oneshot responder.
        respond_to: oneshot::Sender<Result<(), Report<PluginError>>>,
        /// If `Some`, additionally fire hooks from this session's per-session plugins.
        target_session: Option<crate::session_registry::SessionRegistryId>,
    },
    /// Fire all hooks, collect return values (async).
    Collect {
        /// Hook name to fire.
        hook: String,
        /// Serialized hook ctx.
        ctx_json: serde_json::Value,
        /// Oneshot responder.
        respond_to: oneshot::Sender<Result<Vec<serde_json::Value>, Report<PluginError>>>,
        /// If `Some`, additionally fire hooks from this session's per-session plugins.
        target_session: Option<crate::session_registry::SessionRegistryId>,
    },
    /// Fire all hooks, collect return values (sync, blocking caller).
    SyncCollect {
        /// Hook name to fire.
        hook: String,
        /// Serialized hook ctx.
        ctx_json: serde_json::Value,
        /// Oneshot responder (sync caller will `blocking_recv()`).
        respond_to: oneshot::Sender<Result<Vec<serde_json::Value>, Report<PluginError>>>,
        /// If `Some`, additionally fire hooks from this session's per-session plugins.
        target_session: Option<crate::session_registry::SessionRegistryId>,
    },
    /// Load attachable plugins into a new per-session Lua state.
    ///
    /// Sent by `AsyncPluginHandle::create_session_registry`. The thread
    /// allocates a fresh `mlua::Lua`, loads the named attachable plugins,
    /// and stores the resulting hooks map keyed by `registry_id`.
    LoadSession {
        /// Registry ID returned by `create_session_registry`.
        registry_id: crate::session_registry::SessionRegistryId,
        /// Names of attachable plugins to load.
        plugin_names: Vec<String>,
        /// Responder. `Ok` indicates the Lua state is ready.
        respond_to: oneshot::Sender<Result<(), Report<PluginError>>>,
    },
    /// Drop a per-session Lua state and free its memory.
    DestroySession {
        /// Registry ID previously returned by `create_session_registry`.
        registry_id: crate::session_registry::SessionRegistryId,
    },
}

/// Handle for firing async hooks. Send, Sync, and Clone.
///
/// Cloning is cheap — just clones a channel sender and an `Arc<DashMap>`.
#[derive(Clone)]
pub struct AsyncPluginHandle {
    /// Channel sender for async plugin jobs.
    tx: kanal::AsyncSender<PluginJob>,
    /// Shared plugin data store.
    plugin_data: PluginData,
}

impl AsyncPluginHandle {
    /// Construct an async plugin handle from its owned parts.
    ///
    /// Called by [`crate::PluginSystem::build`] to wire the async
    /// background-thread sender and shared plugin-data store.
    pub(crate) fn new(tx: kanal::AsyncSender<PluginJob>, plugin_data: PluginData) -> Self {
        Self { tx, plugin_data }
    }
}

impl AsyncPluginHandle {
    /// Fire an async hook on the background thread.
    ///
    /// Equivalent to `fire_async_for_session(None, hook, ctx)`.
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
        self.fire_async_for_session(None, hook, ctx).await
    }

    /// Fire an async hook, optionally scoped to a session's attached plugins.
    ///
    /// When `target_session` is `None`, only global plugins fire.
    /// When `Some(id)`, both global plugins and that session's per-session
    /// plugins fire.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead, the call times out,
    /// or a hook errors.
    pub async fn fire_async_for_session<T: Serialize>(
        &self,
        target_session: Option<crate::session_registry::SessionRegistryId>,
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
                target_session,
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
    ///
    /// Equivalent to `fire_async_collect_for_session(None, hook, ctx)`.
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
        self.fire_async_collect_for_session(None, hook, ctx).await
    }

    /// Fire an async hook, collecting return values, optionally scoped to a session.
    ///
    /// See [`fire_async_for_session`](Self::fire_async_for_session) for
    /// session-scoping semantics.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead, the call times out,
    /// or a hook errors.
    pub async fn fire_async_collect_for_session<T: Serialize, R: DeserializeOwned>(
        &self,
        target_session: Option<crate::session_registry::SessionRegistryId>,
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
                target_session,
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

    /// Allocate a per-session Lua state on the plugin thread and load the
    /// named attachable plugins into it.
    ///
    /// Returns an opaque [`SessionRegistryId`] used in subsequent
    /// `*_for_session` calls. The Lua state lives until `destroy_session_registry`
    /// is called or the plugin thread exits.
    ///
    /// # Errors
    ///
    /// Returns an error if any of the named plugins cannot be found or
    /// fails to load.
    pub async fn create_session_registry_impl(
        &self,
        plugin_names: Vec<String>,
    ) -> Result<crate::session_registry::SessionRegistryId, Report<PluginError>> {
        let registry_id = crate::session_registry::SessionRegistryId::new();
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(PluginJob::LoadSession {
                registry_id,
                plugin_names,
                respond_to,
            })
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("failed to send LoadSession job to plugin thread")?;
        tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("timed out waiting for plugin thread response (30s)")?
            .map_err(|_e| Report::new(PluginError))
            .attach("plugin thread dropped oneshot responder")??;
        Ok(registry_id)
    }

    /// Drop a per-session Lua state.
    ///
    /// After this call, the registry ID is invalid; using it in a `*_for_session`
    /// call will result in the hooks from that session being silently absent
    /// (no error).
    ///
    /// # Errors
    ///
    /// Returns an error only if the plugin thread is dead.
    pub async fn destroy_session_registry_impl(
        &self,
        registry_id: crate::session_registry::SessionRegistryId,
    ) -> Result<(), Report<PluginError>> {
        self.tx
            .send(PluginJob::DestroySession { registry_id })
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("failed to send DestroySession job to plugin thread")?;
        Ok(())
    }

    /// Get a snapshot of a plugin's data.
    #[must_use]
    pub fn get_plugin_data(&self, plugin_name: &str) -> Option<serde_json::Value> {
        self.plugin_data.get(plugin_name)
    }
}

#[async_trait::async_trait]
impl jinn_domain::feat::plugin_system::SessionPluginRegistry for AsyncPluginHandle {
    async fn create_session_registry(
        &self,
        plugin_names: Vec<String>,
    ) -> Result<
        jinn_domain::feat::plugin_system::SessionRegistryId,
        Report<jinn_domain::feat::plugin_system::SessionPluginRegistryError>,
    > {
        AsyncPluginHandle::create_session_registry_impl(self, plugin_names)
            .await
            .map_err(|_e| Report::new(jinn_domain::feat::plugin_system::SessionPluginRegistryError))
            .attach("create per-session plugin registry")
    }

    async fn destroy_session_registry(
        &self,
        registry_id: jinn_domain::feat::plugin_system::SessionRegistryId,
    ) -> Result<(), Report<jinn_domain::feat::plugin_system::SessionPluginRegistryError>> {
        AsyncPluginHandle::destroy_session_registry_impl(self, registry_id)
            .await
            .map_err(|_e| Report::new(jinn_domain::feat::plugin_system::SessionPluginRegistryError))
            .attach("destroy per-session plugin registry")
    }

    fn name(&self) -> &'static str {
        "AsyncPluginHandle"
    }
}
