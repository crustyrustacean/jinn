//! Async-side plugin handle — sends hook fire requests to the background thread.
//!
//! [`AsyncPluginHandle`] is `Send + Sync + Clone`. It holds only a channel
//! sender and the shared plugin data store. The actual Lua execution happens
//! on a dedicated background thread (see `async_thread.rs`).

use crate::SessionId;
use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::oneshot;
use wherror::Error;

use super::plugin_data::PluginData;

/// Result of creating a per-session plugin registry.
#[derive(Debug)]
pub struct CreateSessionRegistryResult {
    /// The newly created registry ID.
    pub registry_id: super::session_registry::SessionRegistryId,
    /// Tool definitions extracted from the loaded plugins.
    pub tool_metadata: Vec<super::tool_def::PluginToolMetadata>,
}

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
        target_session: Option<super::session_registry::SessionRegistryId>,
        /// Plugin names that are currently enabled. When non-empty,
        /// only plugins in this list will have their hooks fired.
        enabled_plugins: Vec<String>,
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
        target_session: Option<super::session_registry::SessionRegistryId>,
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
        target_session: Option<super::session_registry::SessionRegistryId>,
    },
    /// Load attachable plugins into a new per-session Lua state.
    ///
    /// Sent by `AsyncPluginHandle::create_session_registry`. The thread
    /// allocates a fresh `mlua::Lua`, loads the named attachable plugins,
    /// and stores the resulting hooks map keyed by `registry_id`.
    LoadSession {
        /// Registry ID returned by `create_session_registry`.
        registry_id: super::session_registry::SessionRegistryId,
        /// Names of attachable plugins to load.
        plugin_names: Vec<String>,
        /// The session that owns these plugins. Used for plugin_data scoping in tool handlers.
        origin_session_id: SessionId,
        /// Responder. Returns tool definitions extracted from loaded plugins.
        respond_to:
            oneshot::Sender<Result<Vec<super::tool_def::PluginToolMetadata>, Report<PluginError>>>,
    },
    /// Execute a plugin-defined tool handler.
    ///
    /// Routes to the correct Lua state (global or per-session),
    /// finds the tool handler, calls it with arguments, returns result string.
    ExecuteTool {
        /// If `Some`, use the per-session Lua state; otherwise global.
        target: Option<super::session_registry::SessionRegistryId>,
        /// Domain session ID for plugin_data scoping.
        session_id: SessionId,
        /// Plugin that defined this tool.
        plugin_name: String,
        /// Tool name to execute.
        tool_name: String,
        /// Arguments from the LLM tool call.
        arguments: serde_json::Value,
        /// Oneshot responder. Returns the tool result string.
        respond_to: oneshot::Sender<Result<String, Report<PluginError>>>,
    },
    DestroySession {
        /// Registry ID previously returned by `create_session_registry`.
        registry_id: super::session_registry::SessionRegistryId,
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
        self.fire_async_for_session(None, hook, ctx, vec![]).await
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
        target_session: Option<super::session_registry::SessionRegistryId>,
        hook: &str,
        ctx: &T,
        enabled_plugins: Vec<String>,
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
                enabled_plugins,
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
        target_session: Option<super::session_registry::SessionRegistryId>,
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
        origin_session_id: SessionId,
    ) -> Result<CreateSessionRegistryResult, Report<PluginError>> {
        let registry_id = super::session_registry::SessionRegistryId::new();
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(PluginJob::LoadSession {
                registry_id,
                plugin_names,
                origin_session_id,
                respond_to,
            })
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("failed to send LoadSession job to plugin thread")?;
        let tool_metadata = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("timed out waiting for plugin thread response (30s)")?
            .map_err(|_e| Report::new(PluginError))
            .attach("plugin thread dropped oneshot responder")??;
        Ok(CreateSessionRegistryResult {
            registry_id,
            tool_metadata,
        })
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
        registry_id: super::session_registry::SessionRegistryId,
    ) -> Result<(), Report<PluginError>> {
        self.tx
            .send(PluginJob::DestroySession { registry_id })
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("failed to send DestroySession job to plugin thread")?;
        Ok(())
    }

    /// Set a plugin's data (replaces the entire value).
    pub fn set_plugin_data(&self, plugin_name: &str, value: serde_json::Value) {
        self.plugin_data.set(plugin_name, value);
    }
    /// Set a plugin's data scoped to a session (replaces the entire value).
    ///
    /// This is the write-side counterpart of the session-scoped read used by
    /// sync hooks (e.g. the chat-input badge). Async hooks write via the
    /// Lua `ctx.merge_plugin_data`/`ctx.set_plugin_data` bindings, which also
    /// scope to the hook's session.
    pub fn set_plugin_data_for_session(
        &self,
        session_id: &SessionId,
        plugin_name: &str,
        value: serde_json::Value,
    ) {
        self.plugin_data
            .set_for_session(Some(session_id), plugin_name, value);
    }

    /// Get a snapshot of a plugin's data (no session scope — for global plugins).
    #[must_use]
    pub fn get_plugin_data(&self, plugin_name: &str) -> Option<serde_json::Value> {
        self.plugin_data.get(plugin_name)
    }

    /// Get a snapshot of a plugin's data scoped to a session.
    #[must_use]
    pub fn get_plugin_data_for_session(
        &self,
        session_id: &SessionId,
        plugin_name: &str,
    ) -> Option<serde_json::Value> {
        self.plugin_data
            .get_for_session(Some(session_id), plugin_name)
    }
    /// Execute a plugin-defined tool handler on the background thread.
    ///
    /// Routes to the correct Lua state (global or per-session),
    /// finds the tool handler by plugin + tool name, builds a ctx,
    /// calls the handler with `(ctx, arguments)`, returns the result string.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead, the tool handler is not found,
    /// or the handler itself errors.
    pub async fn execute_tool(
        &self,
        target: Option<super::session_registry::SessionRegistryId>,
        session_id: SessionId,
        plugin_name: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String, Report<PluginError>> {
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(PluginJob::ExecuteTool {
                target,
                session_id,
                plugin_name: plugin_name.to_owned(),
                tool_name: tool_name.to_owned(),
                arguments: arguments.clone(),
                respond_to,
            })
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("failed to send ExecuteTool job to plugin thread")
            .attach(plugin_name.to_owned())?;
        tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_e| Report::new(PluginError))
            .attach("plugin tool execution timed out")
            .attach(plugin_name.to_owned())?
            .map_err(|_e| Report::new(PluginError))
            .attach("plugin tool response dropped")
            .attach(plugin_name.to_owned())?
    }
}

#[async_trait::async_trait]
impl crate::feat::plugin_system::SessionPluginRegistry for AsyncPluginHandle {
    async fn create_session_registry(
        &self,
        plugin_names: Vec<String>,
        origin_session_id: SessionId,
    ) -> Result<
        crate::feat::plugin_system::CreateSessionRegistryResult,
        Report<crate::feat::plugin_system::SessionPluginRegistryError>,
    > {
        let result =
            AsyncPluginHandle::create_session_registry_impl(self, plugin_names, origin_session_id)
                .await
                .map_err(|_e| Report::new(crate::feat::plugin_system::SessionPluginRegistryError))
                .attach("create per-session plugin registry")?;
        Ok(crate::feat::plugin_system::CreateSessionRegistryResult {
            registry_id: result.registry_id,
            tool_metadata: result
                .tool_metadata
                .into_iter()
                .map(std::convert::Into::into)
                .collect(),
        })
    }

    async fn destroy_session_registry(
        &self,
        registry_id: crate::feat::plugin_system::SessionRegistryId,
    ) -> Result<(), Report<crate::feat::plugin_system::SessionPluginRegistryError>> {
        AsyncPluginHandle::destroy_session_registry_impl(self, registry_id)
            .await
            .map_err(|_e| Report::new(crate::feat::plugin_system::SessionPluginRegistryError))
            .attach("destroy per-session plugin registry")
    }

    fn name(&self) -> &'static str {
        "AsyncPluginHandle"
    }
}
