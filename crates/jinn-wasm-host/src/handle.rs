//! Sync-side handle for firing async hooks on the background WASM thread.
//!
//! [`AsyncWasmHandle`] is `Send + Sync + Clone`. It holds only an async channel
//! sender and shared references to the bag stores. The actual WASM execution
//! happens on a dedicated background thread inside a `tokio::LocalSet` (see
//! `async_thread.rs`).

use std::time::Duration;

use error_stack::{Report, ResultExt};
use jinn_core_types::SessionId;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::async_thread::{AsyncPluginError, AsyncThreadSender, WasmJob};
use crate::bag::{GlobalBagStore, InstanceBagStore};

/// A session-attached plugin registry identifier.
pub type SessionRegistryId = jinn_core_types::SessionRegistryId;
/// A plugin instance identifier.
pub type PluginInstanceId = jinn_core_types::PluginInstanceId;

/// Result of creating a per-session plugin registry.
#[derive(Debug)]
pub struct CreateSessionRegistryResult {
    pub registry_id: SessionRegistryId,
    pub tool_metadata: Vec<WasmToolMetadata>,
}

/// One tool declared by a loaded plugin (extracted from its manifest).
#[derive(Debug, Clone)]
pub struct WasmToolMetadata {
    pub name: String,
    pub description: String,
    /// Full JSON Schema for parameters.
    pub parameters: serde_json::Value,
    /// Plugin that defines this tool.
    pub plugin_name: String,
    /// Whether this tool is global or session-attached.
    pub scope: jinn_domain::feat::plugin_dispatch::ToolScope,
}

/// Handle for firing async hooks. Send, Sync, and Clone.
///
/// Cloning is cheap — it clones the channel sender and the `Arc` bag stores.
/// The background thread owns the `!Send` `StoreSet`; this handle just ships
/// jobs to it over an async channel.
#[derive(Clone)]
pub struct AsyncWasmHandle {
    tx: AsyncThreadSender,
    bags: InstanceBagStore,
    globals: GlobalBagStore,
}

impl std::fmt::Debug for AsyncWasmHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncWasmHandle").finish_non_exhaustive()
    }
}

const TIMEOUT: Duration = Duration::from_secs(30);

impl AsyncWasmHandle {
    /// Construct from owned parts. Called by [`crate::system::build`].
    #[must_use]
    pub fn new(tx: AsyncThreadSender, bags: InstanceBagStore, globals: GlobalBagStore) -> Self {
        Self { tx, bags, globals }
    }

    /// Fire an async hook (global plugins only), discarding return values.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead, the call times out,
    /// or a hook traps.
    pub async fn fire_async(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<(), Report<AsyncPluginError>> {
        self.fire_async_for_session(None, hook, ctx, None).await
    }

    /// Fire an async hook, optionally scoped to a session's attached plugins.
    ///
    /// When `target_session` is `None`, only global plugins fire. When
    /// `Some`, both global plugins and that session's per-session plugins
    /// fire (filtered by `enabled_instances` if provided).
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead, the call times out,
    /// or a hook traps.
    pub async fn fire_async_for_session(
        &self,
        target_session: Option<SessionRegistryId>,
        hook: &str,
        ctx: &Value,
        enabled_instances: Option<Vec<PluginInstanceId>>,
    ) -> Result<(), Report<AsyncPluginError>> {
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(WasmJob::Fire {
                hook: hook.to_owned(),
                ctx_json: ctx.clone(),
                respond_to,
                target_session,
                enabled_instances,
            })
            .await
            .change_context(AsyncPluginError)
            .attach("failed to send Fire job to wasm thread")
            .attach(hook.to_owned())?;

        tokio::time::timeout(TIMEOUT, rx)
            .await
            .change_context(AsyncPluginError)
            .attach("timed out waiting for wasm thread response (30s)")
            .attach(hook.to_owned())?
            .map_err(|_| Report::new(AsyncPluginError).attach("wasm thread dropped responder"))
            .attach(hook.to_owned())??;
        Ok(())
    }

    /// Fire an async hook, collecting return values from all global plugins.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead, the call times out,
    /// or a hook traps.
    pub async fn fire_async_collect(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<AsyncPluginError>> {
        self.fire_async_collect_for_session(None, hook, ctx).await
    }

    /// Fire an async hook, collecting values from globals + a session's plugins.
    ///
    /// # Errors
    ///
    /// Returns an error if the background thread is dead, the call times out,
    /// or a hook traps.
    pub async fn fire_async_collect_for_session(
        &self,
        target_session: Option<SessionRegistryId>,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<AsyncPluginError>> {
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(WasmJob::Collect {
                hook: hook.to_owned(),
                ctx_json: ctx.clone(),
                respond_to,
                target_session,
            })
            .await
            .change_context(AsyncPluginError)
            .attach("failed to send Collect job to wasm thread")?;

        let results: Vec<Value> = tokio::time::timeout(TIMEOUT, rx)
            .await
            .change_context(AsyncPluginError)
            .attach("timed out waiting for wasm thread response (30s)")?
            .map_err(|_| Report::new(AsyncPluginError).attach("wasm thread dropped responder"))??;
        Ok(results)
    }

    /// Set a global plugin's data bag (replaces the entire value).
    pub fn set_plugin_data(&self, plugin_name: &str, value: Vec<u8>) {
        self.bags.set(plugin_name, value);
    }

    /// Set a session-scoped plugin's data bag.
    pub fn set_plugin_data_for_session(
        &self,
        session_id: &SessionId,
        instance_id: &PluginInstanceId,
        value: Vec<u8>,
    ) {
        self.bags.set_for_session(session_id, instance_id, value);
    }

    /// Get a snapshot of a global plugin's data bag.
    #[must_use]
    pub fn get_plugin_data(&self, plugin_name: &str) -> Option<Vec<u8>> {
        self.bags.get(plugin_name)
    }

    /// Get a snapshot of a session-scoped plugin's data bag.
    #[must_use]
    pub fn get_plugin_data_for_session(
        &self,
        session_id: &SessionId,
        instance_id: &PluginInstanceId,
    ) -> Option<Vec<u8>> {
        self.bags.get_for_session(session_id, instance_id)
    }
}

// ─── Trait impls: AsyncWasmHandle as a PluginFire + SessionPluginRegistry backend ───
//
// These bridge the domain DI traits to the WASM handles. The fire_* methods
// delegate to the existing channel-backed jobs. Tool execution and
// session-registry management require additional WasmJob variants that are
// added alongside the full runtime wiring (judge plugin port); for now they
// return a clear "not wired" error so the Services container compiles and the
// fire-path works end to end.

use jinn_domain::feat::plugin_dispatch::{
    PluginFire, PluginFireError, SessionPluginRegistry, SessionPluginRegistryError,
};

#[async_trait::async_trait]
impl PluginFire for AsyncWasmHandle {
    async fn fire_async_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<(), Report<PluginFireError>> {
        self.fire_async(hook, ctx)
            .await
            .map_err(|r| r.change_context(PluginFireError))
    }

    async fn fire_async_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
        enabled_instances: Option<Vec<PluginInstanceId>>,
    ) -> Result<(), Report<PluginFireError>> {
        self.fire_async_for_session(Some(session), hook, ctx, enabled_instances)
            .await
            .map_err(|r| r.change_context(PluginFireError))
    }

    async fn fire_async_collect_json(
        &self,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginFireError>> {
        self.fire_async_collect(hook, ctx)
            .await
            .map_err(|r| r.change_context(PluginFireError))
    }

    async fn fire_async_collect_for_session_json(
        &self,
        session: SessionRegistryId,
        hook: &str,
        ctx: &Value,
    ) -> Result<Vec<Value>, Report<PluginFireError>> {
        self.fire_async_collect_for_session(Some(session), hook, ctx)
            .await
            .map_err(|r| r.change_context(PluginFireError))
    }

    async fn execute_plugin_tool(
        &self,
        target: Option<SessionRegistryId>,
        session_id: &SessionId,
        parent_session_id: Option<&SessionId>,
        plugin_name: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<String, Report<PluginFireError>> {
        let (respond_to, rx) = oneshot::channel();
        let ctx_json = serde_json::json!({
            "session_id": session_id,
            "parent_session_id": parent_session_id,
            "plugin_name": plugin_name,
        });
        self.tx
            .send(WasmJob::ExecuteTool {
                target_session: target,
                plugin_name: plugin_name.to_owned(),
                tool_name: tool_name.to_owned(),
                arguments: arguments.to_string(),
                ctx_json,
                respond_to,
            })
            .await
            .change_context(PluginFireError)
            .attach("failed to send ExecuteTool job to wasm thread")?;

        match tokio::time::timeout(TIMEOUT, rx).await {
            Err(_) => Err(Report::new(PluginFireError)
                .attach("timed out waiting for wasm tool execution (30s)")),
            Ok(Err(_recv)) => {
                Err(Report::new(PluginFireError).attach("wasm thread dropped responder"))
            }
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(report))) => Err(report.change_context(PluginFireError)),
        }
    }

    fn name(&self) -> &'static str {
        "AsyncWasmHandle"
    }
}

#[async_trait::async_trait]
impl SessionPluginRegistry for AsyncWasmHandle {
    async fn create_session_registry(
        &self,
        instances: Vec<(PluginInstanceId, String)>,
        origin_session_id: SessionId,
    ) -> Result<
        jinn_domain::feat::plugin_dispatch::CreateSessionRegistryResult,
        Report<SessionPluginRegistryError>,
    > {
        let registry_id = SessionRegistryId::new();
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(WasmJob::LoadSession {
                registry_id,
                instances: instances.clone(),
                origin_session_id: origin_session_id.clone(),
                respond_to,
            })
            .await
            .change_context(SessionPluginRegistryError)
            .attach("failed to send LoadSession job to wasm thread")?;

        let tool_metadata = match tokio::time::timeout(TIMEOUT, rx).await {
            // Outer Err: the 30s deadline elapsed.
            Err(_) => {
                return Err(Report::new(SessionPluginRegistryError)
                    .attach("timed out waiting for wasm thread response (30s)"));
            }
            // oneshot sender dropped — thread died before responding.
            Ok(Err(_recv)) => {
                return Err(
                    Report::new(SessionPluginRegistryError).attach("wasm thread dropped responder")
                );
            }
            Ok(Ok(inner)) => match inner {
                Ok(tools) => tools,
                Err(report) => {
                    return Err(report.change_context(SessionPluginRegistryError));
                }
            },
        };

        Ok(
            jinn_domain::feat::plugin_dispatch::CreateSessionRegistryResult {
                registry_id,
                tool_metadata: tool_metadata
                    .into_iter()
                    .map(|t| jinn_domain::feat::plugin_dispatch::PluginToolMetadata {
                        name: t.name,
                        description: t.description,
                        parameters: t.parameters,
                        plugin_name: t.plugin_name,
                        scope: t.scope,
                    })
                    .collect(),
            },
        )
    }

    async fn destroy_session_registry(
        &self,
        registry_id: SessionRegistryId,
    ) -> Result<(), Report<SessionPluginRegistryError>> {
        self.tx
            .send(WasmJob::DestroySession { registry_id })
            .await
            .change_context(SessionPluginRegistryError)
            .attach("failed to send DestroySession job to wasm thread")?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "AsyncWasmHandle"
    }
}
