//! The dedicated background thread that owns the async `StoreSet`.
//!
//! `wasmtime::Store` is `!Send`, so async hooks (which may `.await` on host
//! imports like `request-llm-oneshot`) cannot run on the tokio multi-thread
//! runtime directly. Instead, this crate spawns a dedicated OS thread running
//! a `tokio::current_thread::Runtime` + `LocalSet`. The `!Send` store futures
//! are polled there.
//!
//! This is the direct WASM analogue of the old Lua system's
//! `async_thread.rs`: a single-threaded loop consuming a `WasmJob` channel.
//!
//! # Sync callers
//!
//! Sync render-thread hooks also need to fire hooks, but they run on the
//! render thread's own store set (the dual-store model) — they never cross
//! into this background thread. This thread only services async jobs.

use std::collections::HashMap;
use std::thread::JoinHandle;

use error_stack::Report;
use jinn_core_types::{PluginInstanceId, SessionId, SessionRegistryId};
use serde_json::Value;
use tokio::sync::oneshot;
use wherror::Error;

use crate::bag::{GlobalBagStore, InstanceBagStore};
use crate::engine::WasmEngine;
use crate::loader::LoadedStore;

/// Error for WASM plugin system failures.
#[derive(Debug, Error)]
#[error(debug)]
pub struct AsyncPluginError;

/// Internal message sent to the background WASM thread.
///
/// Mirrors the old `PluginJob`. Each variant carries an oneshot responder.
pub enum WasmJob {
    /// Fire all hooks for a name, discarding return values.
    Fire {
        hook: String,
        ctx: jinn_domain::feat::plugin_dispatch::HookCtx,
        respond_to: oneshot::Sender<Result<(), Report<AsyncPluginError>>>,
        target_session: Option<SessionRegistryId>,
        enabled_instances: Option<Vec<PluginInstanceId>>,
    },
    /// Fire all hooks for a name, collecting return values (sync caller blocks).
    SyncCollect {
        hook: String,
        ctx_json: Value,
        respond_to: oneshot::Sender<Result<Vec<Value>, Report<AsyncPluginError>>>,
        target_session: Option<SessionRegistryId>,
    },
    /// Instantiate attachable plugins into a new per-session store set slot.
    LoadSession {
        registry_id: SessionRegistryId,
        instances: Vec<(PluginInstanceId, String)>,
        origin_session_id: SessionId,
        respond_to:
            oneshot::Sender<Result<Vec<crate::handle::WasmToolMetadata>, Report<AsyncPluginError>>>,
    },
    /// Execute a plugin-defined tool via `run-tool`. Returns the tool result string.
    ExecuteTool {
        target_session: Option<SessionRegistryId>,
        plugin_name: String,
        tool_name: String,
        arguments: String,
        session_id: SessionId,
        parent_session_id: Option<SessionId>,
        respond_to: oneshot::Sender<Result<String, Report<AsyncPluginError>>>,
    },
    /// Drop a per-session store set slot.
    DestroySession { registry_id: SessionRegistryId },
}

/// Async channel sender for [`WasmJob`]. `Send`, cloneable.
pub(crate) type AsyncThreadSender = kanal::AsyncSender<WasmJob>;
/// Sync channel sender for [`WasmJob`] — derived via `clone_sync()` so the
/// async sender stays valid. Used by [`crate::sync_handle::SyncWasmHandle`]
/// for blocking sync hook calls from actor threads.
pub(crate) type SyncThreadSender = kanal::Sender<WasmJob>;
type AsyncThreadReceiver = kanal::AsyncReceiver<WasmJob>;

/// Handle to the running background thread.
pub struct AsyncThreadHandle {
    _join: JoinHandle<()>,
}

/// State owned by the background thread. `!Send` (holds `StoreSet`).
struct ThreadState {
    /// The global (non-session) store set.
    global_store: LoadedStore,
    /// Per-session store sets, keyed by registry id.
    session_stores: HashMap<SessionRegistryId, LoadedStore>,
    /// Which instances belong to which registry (for DestroySession).
    session_instances: HashMap<SessionRegistryId, Vec<(PluginInstanceId, String)>>,
    /// Compiled attachable plugins, keyed by name — used to instantiate
    /// per-session stores when a session registry is created.
    attachable_components: HashMap<String, crate::loader::CompiledPlugin>,
    /// Shared engine + linker for per-session instantiation.
    engine: crate::engine::WasmEngine,
    linker: wasmtime::component::Linker<crate::store::StoreState>,
    /// Shared bag layer (Arc-backed; cheap clone).
    bags: crate::bag::InstanceBagStore,
    globals: crate::bag::GlobalBagStore,
    /// Host-import callbacks (`emit`, `request-*`). Cloned into every store
    /// so that `host::emit` / `host::request-llm-oneshot` reach the domain
    /// bridge on the async thread, not just the sync render-thread store.
    host_imports: crate::imports::HostImports,
}

impl AsyncThreadHandle {
    /// Spawn the background thread.
    ///
    /// Returns the channel sender (for shipping jobs) and the join handle.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        engine: WasmEngine,
        bags: InstanceBagStore,
        globals: GlobalBagStore,
        linker: wasmtime::component::Linker<crate::store::StoreState>,
        _runtime_handle: tokio::runtime::Handle,
        plugins: Vec<crate::loader::CompiledPlugin>,
        host_imports: crate::imports::HostImports,
    ) -> (AsyncThreadSender, SyncThreadSender, AsyncThreadHandle) {
        let (tx, rx) = kanal::unbounded_async::<WasmJob>();
        let sync_tx = tx.clone_sync();

        let join = std::thread::Builder::new()
            .name("jinn-wasm-plugin".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to build wasm plugin runtime");
                        return;
                    }
                };

                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, async move {
                    let mut global_store = crate::loader::new_loaded_store(
                        crate::store::StoreKind::Async,
                        &engine,
                        &bags,
                        &globals,
                    );
                    global_store.storeset.set_imports(host_imports.clone());

                    // Partition: globals load immediately into the global
                    // store set; attachables are held compiled, instantiated
                    // per-session when a registry is created.
                    let mut attachable_components = HashMap::new();
                    for plugin in &plugins {
                        match plugin.meta.kind {
                            crate::discovery::PluginKind::Global => {
                                if let Err(e) = crate::loader::load_globals_into_async(
                                    &mut global_store,
                                    std::slice::from_ref(plugin),
                                    &linker,
                                ).await {
                                    tracing::warn!(?e, plugin = %plugin.meta.name, "failed to load global plugin");
                                }
                            }
                            crate::discovery::PluginKind::Attachable => {
                                attachable_components.insert(plugin.meta.name.clone(), plugin.clone());
                            }
                        }
                    }

                    let state = ThreadState {
                        global_store,
                        session_stores: HashMap::new(),
                        session_instances: HashMap::new(),
                        attachable_components,
                        engine,
                        linker,
                        bags,
                        globals,
                        host_imports,
                    };
                    thread_loop(rx, state).await;
                });
            })
            .expect("spawn jinn-wasm-plugin thread");

        (tx, sync_tx, AsyncThreadHandle { _join: join })
    }
}

/// Drive the background thread, executing each received [`WasmJob`] in turn.
async fn thread_loop(rx: AsyncThreadReceiver, mut state: ThreadState) {
    while let Ok(job) = rx.recv().await {
        execute_job(&mut state, job).await;
    }
}

/// Execute a single job against the thread state.
async fn execute_job(state: &mut ThreadState, job: WasmJob) {
    match job {
        WasmJob::Fire {
            hook,
            ctx,
            respond_to,
            target_session,
            enabled_instances,
        } => {
            let result = fire_hooks(state, target_session, &hook, &ctx, enabled_instances)
                .await
                .map(|_| ());
            let _ = respond_to.send(result);
        }
        WasmJob::LoadSession {
            registry_id,
            instances,
            origin_session_id,
            respond_to,
        } => {
            let result = load_session(state, registry_id, instances, origin_session_id).await;
            let _ = respond_to.send(result);
        }
        WasmJob::ExecuteTool {
            target_session,
            plugin_name,
            tool_name,
            arguments,
            session_id,
            parent_session_id,
            respond_to,
        } => {
            let result = execute_tool(
                state,
                target_session,
                &plugin_name,
                &tool_name,
                &arguments,
                &session_id,
                parent_session_id.as_ref(),
            )
            .await;
            let _ = respond_to.send(result);
        }
        WasmJob::DestroySession { registry_id } => {
            state.session_stores.remove(&registry_id);
            state.session_instances.remove(&registry_id);
        }
        WasmJob::SyncCollect {
            hook,
            ctx_json,
            respond_to,
            target_session,
        } => {
            let result = fire_hooks_collect_json(state, target_session, &hook, &ctx_json).await;
            let _ = respond_to.send(result);
        }
    }
}

/// Fire a hook on all matching plugins, discarding return values.
///
/// Matches the old Lua semantics: global plugins always fire; when
/// `target_session` is set, that session's attached plugins also fire
/// (filtered by `enabled_instances`).
async fn fire_hooks(
    state: &mut ThreadState,
    target_session: Option<SessionRegistryId>,
    hook: &str,
    ctx: &jinn_domain::feat::plugin_dispatch::HookCtx,
    enabled_instances: Option<Vec<PluginInstanceId>>,
) -> Result<(), Report<AsyncPluginError>> {
    tracing::debug!(
        %hook,
        global_count = state.global_store.storeset.len(),
        session_count = state.session_stores.values().map(|s| s.storeset.len()).sum::<usize>(),
        target = ?target_session,
        enabled = ?enabled_instances.as_ref().map(Vec::len),
        "fire_hooks"
    );
    // Fire on global plugins.
    fire_on_store(&mut state.global_store, hook, ctx).await;

    // Fire on the session's plugins, if any.
    if let Some(rid) = target_session
        && let Some(session_store) = state.session_stores.get_mut(&rid)
    {
        if let Some(enabled) = &enabled_instances {
            fire_on_store_filtered(session_store, hook, ctx, enabled).await;
        } else {
            fire_on_store(session_store, hook, ctx).await;
        }
    }
    Ok(())
}

/// Fire a hook on all matching plugins, collecting return values.
async fn fire_hooks_collect(
    state: &mut ThreadState,
    target_session: Option<SessionRegistryId>,
    hook: &str,
    ctx: &jinn_domain::feat::plugin_dispatch::HookCtx,
) -> Result<Vec<Value>, Report<AsyncPluginError>> {
    let mut results = collect_from_store(&mut state.global_store, hook, ctx).await;
    if let Some(rid) = target_session
        && let Some(session_store) = state.session_stores.get_mut(&rid)
    {
        results.extend(collect_from_store(session_store, hook, ctx).await);
    }
    Ok(results)
}

/// Execute a plugin tool by resolving the named instance then calling `run-tool`.
///
/// Looks up the instance by `plugin_name` in the session store (if `target_session`
/// is set) or the global store. Returns the tool result string.
async fn execute_tool(
    state: &mut ThreadState,
    target_session: Option<SessionRegistryId>,
    plugin_name: &str,
    tool_name: &str,
    arguments: &str,
    session_id: &SessionId,
    parent_session_id: Option<&SessionId>,
) -> Result<String, Report<AsyncPluginError>> {
    let store = match target_session {
        Some(rid) => state
            .session_stores
            .get_mut(&rid)
            .ok_or_else(|| Report::new(AsyncPluginError).attach("session store not found"))?,
        None => &mut state.global_store,
    };
    let inst = store
        .storeset
        .iter_mut()
        .find(|i| i.ctx().plugin_name == plugin_name)
        .ok_or_else(|| {
            Report::new(AsyncPluginError)
                .attach("plugin instance not found")
                .attach(plugin_name.to_owned())
        })?;
    crate::dispatch::dispatch_run_tool(inst, tool_name, arguments, session_id, parent_session_id)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, %plugin_name, %tool_name, "tool execution failed");
            Report::new(AsyncPluginError).attach("tool execution failed")
        })
}

/// Instantiate the named attachable plugins into a fresh per-session store set.
///
/// Each `(instance_id, plugin_name)` pair resolves to a compiled attachable
/// component; if unknown, it's skipped with a warning. Returns the tool
/// metadata from every successfully instantiated plugin's manifest.
async fn load_session(
    state: &mut ThreadState,
    registry_id: SessionRegistryId,
    instances: Vec<(PluginInstanceId, String)>,
    origin_session_id: SessionId,
) -> Result<Vec<crate::handle::WasmToolMetadata>, Report<AsyncPluginError>> {
    let mut session_store = crate::loader::new_loaded_store(
        crate::store::StoreKind::Async,
        &state.engine,
        &state.bags,
        &state.globals,
    );
    session_store
        .storeset
        .set_imports(state.host_imports.clone());
    let mut tool_metadata = Vec::new();

    for (instance_id, plugin_name) in &instances {
        let Some(compiled) = state.attachable_components.get(plugin_name).cloned() else {
            tracing::warn!(%plugin_name, "attachable plugin not compiled; skipping");
            continue;
        };
        let ctx = crate::store::InstanceCtx {
            plugin_name: plugin_name.clone(),
            instance_id: instance_id.clone(),
            session_id: Some(origin_session_id.clone()),
        };
        match session_store
            .storeset
            .load_async(&compiled.component, ctx, &state.linker)
            .await
        {
            Err(e) => {
                tracing::warn!(?e, %plugin_name, "failed to load attachable plugin into session store");
                continue;
            }
            Ok(wit_manifest) => {
                let manifest = crate::loader::convert_manifest(plugin_name, wit_manifest);
                for tool in &manifest.tools {
                    tool_metadata.push(crate::handle::WasmToolMetadata {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: serde_json::Value::Object(serde_json::Map::new()),
                        plugin_name: plugin_name.clone(),
                        scope: if tool.global {
                            jinn_domain::feat::plugin_dispatch::ToolScope::Global
                        } else {
                            jinn_domain::feat::plugin_dispatch::ToolScope::Attached
                        },
                    });
                }
                session_store
                    .manifests
                    .insert(plugin_name.clone(), manifest);
            }
        }
    }

    state.session_stores.insert(registry_id, session_store);
    state.session_instances.insert(registry_id, instances);
    Ok(tool_metadata)
}

/// Fire a hook on every instance in a store set (fire-and-forget).
///
/// Uses the typed dispatcher ([`dispatch_async_hook`]) so well-known hooks
/// resolve to their WIT-typed exports, and plugin-defined hooks fall through
/// to `run-trigger`. Errors per-instance are logged and swallowed — a single
/// failing plugin must not break the fire-and-forget fan-out.
async fn fire_on_store(store: &mut LoadedStore, hook: &str, ctx: &jinn_domain::feat::plugin_dispatch::HookCtx) {
    for inst in store.storeset.iter_mut() {
        tracing::debug!(hook, plugin = %inst.ctx().plugin_name, "firing async hook");
        if let Err(e) = crate::dispatch::dispatch_async_hook(inst, hook, ctx).await {
            tracing::warn!(error = %e, hook, plugin = %inst.ctx().plugin_name, "async hook dispatch failed");
        }
    }
}

/// Fire a hook on instances matching the enabled set.
async fn fire_on_store_filtered(
    store: &mut LoadedStore,
    hook: &str,
    ctx: &jinn_domain::feat::plugin_dispatch::HookCtx,
    enabled: &[PluginInstanceId],
) {
    for inst in store.storeset.iter_mut() {
        if !enabled.iter().any(|id| id == &inst.ctx().instance_id) {
            continue;
        }
        if let Err(e) = crate::dispatch::dispatch_async_hook(inst, hook, ctx).await {
            tracing::warn!(error = %e, hook, plugin = %inst.ctx().plugin_name, "async hook dispatch failed");
        }
    }
}

/// Fire a hook and collect each instance's return value.
///
/// The async hook exports are all fire-and-forget (no return value), so this
/// only collects results for the well-known sync-style hooks routed through
/// the async path. Async hooks contribute nothing; the domain does not rely
/// on return values from async hooks.
async fn collect_from_store(store: &mut LoadedStore, hook: &str, ctx: &jinn_domain::feat::plugin_dispatch::HookCtx) -> Vec<Value> {
    fire_on_store(store, hook, ctx).await;
    Vec::new()
}
/// Fire a hook on all matching plugins, collecting return values (sync/JSON).
///
/// Used only by the `SyncCollect` job — the sync `PluginSyncCall` path which
/// still speaks JSON at the trait seam. Calls `dispatch_sync_hook` so it can
/// collect typed results from the sync render hooks.
async fn fire_hooks_collect_json(
    state: &mut ThreadState,
    target_session: Option<SessionRegistryId>,
    hook: &str,
    ctx_json: &Value,
) -> Result<Vec<Value>, Report<AsyncPluginError>> {
    let mut results = collect_from_store_json(&mut state.global_store, hook, ctx_json);
    if let Some(rid) = target_session
        && let Some(session_store) = state.session_stores.get_mut(&rid)
    {
        results.extend(collect_from_store_json(session_store, hook, ctx_json));
    }
    Ok(results)
}

/// Collect sync hook return values from a store (JSON ctx in, JSON out).
fn collect_from_store_json(store: &mut LoadedStore, hook: &str, ctx_json: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for inst in store.storeset.iter_mut() {
        match crate::dispatch::dispatch_sync_hook(inst, hook, ctx_json) {
            Ok(Some(v)) => out.push(v),
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, hook, plugin = %inst.ctx().plugin_name, "sync hook dispatch failed"),
        }
    }
    out
}
