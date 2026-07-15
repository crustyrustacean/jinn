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

use error_stack::{Report, ResultExt};
use futures::stream::{FuturesOrdered, StreamExt as _};
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
pub(crate) enum WasmJob {
    /// Fire all hooks for a name, discarding return values.
    Fire {
        hook: String,
        ctx_json: Value,
        respond_to: oneshot::Sender<Result<(), Report<AsyncPluginError>>>,
        target_session: Option<SessionRegistryId>,
        enabled_instances: Option<Vec<PluginInstanceId>>,
    },
    Collect {
        hook: String,
        ctx_json: Value,
        respond_to: oneshot::Sender<Result<Vec<Value>, Report<AsyncPluginError>>>,
        target_session: Option<SessionRegistryId>,
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
    /// Drop a per-session store set slot.
    DestroySession {
        registry_id: SessionRegistryId,
    },
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
        _linker: wasmtime::component::Linker<crate::store::StoreState>,
        _runtime_handle: tokio::runtime::Handle,
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
                    let state = ThreadState {
                        global_store: crate::loader::new_loaded_store(
                            crate::store::StoreKind::Async,
                            &engine,
                            &bags,
                            &globals,
                        ),
                        session_stores: HashMap::new(),
                        session_instances: HashMap::new(),
                    };
                    thread_loop(rx, state).await;
                });
            })
            .expect("spawn jinn-wasm-plugin thread");

        (tx, sync_tx, AsyncThreadHandle { _join: join })
    }
}

/// Drive the background thread, executing each received [`WasmJob`] in turn.
async fn thread_loop(mut rx: AsyncThreadReceiver, mut state: ThreadState) {
    while let Some(job) = rx.recv().await.ok() {
        execute_job(&mut state, job).await;
    }
}

/// Execute a single job against the thread state.
async fn execute_job(state: &mut ThreadState, job: WasmJob) {
    match job {
        WasmJob::Fire {
            hook,
            ctx_json,
            respond_to,
            target_session,
            enabled_instances,
        } => {
            let result = fire_hooks(state, target_session, &hook, &ctx_json, enabled_instances)
                .await
                .map(|_| ());
            let _ = respond_to.send(result);
        }
        WasmJob::Collect {
            hook,
            ctx_json,
            respond_to,
            target_session,
        } => {
            let result = fire_hooks_collect(state, target_session, &hook, &ctx_json).await;
            let _ = respond_to.send(result);
        }
        WasmJob::LoadSession {
            registry_id,
            instances,
            origin_session_id: _,
            respond_to,
        } => {
            // Phase 3: instantiate attachable plugins into a per-session store
            // set. For now, record the instance list and return empty tools.
            state.session_instances.insert(registry_id, instances);
            let _ = respond_to.send(Ok(Vec::new()));
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
            // Same as Collect — the caller blocks via blocking_recv().
            let result = fire_hooks_collect(state, target_session, &hook, &ctx_json).await;
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
    _ctx: &Value,
    enabled_instances: Option<Vec<PluginInstanceId>>,
) -> Result<(), Report<AsyncPluginError>> {
    // Fire on global plugins.
    fire_on_store(&mut state.global_store, hook).await;

    // Fire on the session's plugins, if any.
    if let Some(rid) = target_session {
        if let Some(session_store) = state.session_stores.get_mut(&rid) {
            if let Some(enabled) = &enabled_instances {
                fire_on_store_filtered(session_store, hook, enabled).await;
            } else {
                fire_on_store(session_store, hook).await;
            }
        }
    }
    Ok(())
}

/// Fire a hook on all matching plugins, collecting return values.
async fn fire_hooks_collect(
    state: &mut ThreadState,
    target_session: Option<SessionRegistryId>,
    hook: &str,
    _ctx: &Value,
) -> Result<Vec<Value>, Report<AsyncPluginError>> {
    let mut results = collect_from_store(&mut state.global_store, hook).await;
    if let Some(rid) = target_session {
        if let Some(session_store) = state.session_stores.get_mut(&rid) {
            results.extend(collect_from_store(session_store, hook).await);
        }
    }
    Ok(results)
}

/// Fire a hook on every instance in a store set (fire-and-forget).
async fn fire_on_store(store: &mut LoadedStore, hook: &str) {
    // Runtime export lookup: call the export if present, skip if absent.
    // Phase 3 will replace this placeholder body with the typed hook dispatch
    // once the well-known hook → typed-binding mapping is wired.
    for inst in store.storeset.iter_mut() {
        let exists = inst
            .with(|store, instance| {
                use wasmtime::AsContextMut;
                instance.get_func(store.as_context_mut(), hook).is_some()
            });
        if !exists {
            continue;
        }
        // Call the export asynchronously; ignore the result (fire-and-forget).
        let _ = inst
            .with_async(|store, instance| async {
                let Some(func) = instance.get_func(&mut *store, hook) else {
                    return Ok::<(), Report<AsyncPluginError>>(());
                };
                let mut results: Vec<wasmtime::component::Val> = Vec::new();
                let _ = func.call_async(&mut *store, &[], &mut results).await;
                Ok(())
            })
            .await;
    }
}

/// Fire a hook on instances matching the enabled set.
async fn fire_on_store_filtered(store: &mut LoadedStore, hook: &str, enabled: &[PluginInstanceId]) {
    let _ = (store, hook, enabled);
    // TODO Phase 3: iterate only the enabled instances.
}

/// Fire a hook and collect each instance's return value.
async fn collect_from_store(store: &mut LoadedStore, hook: &str) -> Vec<Value> {
    let _ = (store, hook);
    Vec::new()
}
