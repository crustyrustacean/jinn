//! WASM engine — instantiate a guest under granted capabilities.
//!
//! This module runs **inside the runner child** (`jinn --serve-wasm-plugin`):
//! it builds a wasmtime [`Engine`] with epoch interruption and store limits,
//! constructs the WASI context from the granted [`Grants`] (preopened
//! directories, optional `wasi:http`), instantiates the component, and runs
//! its `wasi:cli/command` run function to completion.
//!
//! The guest's stdio *is* the runner child's stdio — jinn's NDJSON pipes —
//! so a Rust guest using the SDK reads host envelopes from stdin and writes
//! plugin messages to stdout directly. This module never interprets wire
//! semantics; it only executes with the right capabilities.

use std::path::Path;

use error_stack::{Report, ResultExt as _};
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p3::bindings::Command;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::WasiHttpCtx;
use wasmtime_wasi_http::p3::WasiHttpView;

use crate::grants::Grants;

/// The engine failed to start or the guest failed to run.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum EngineError {
    /// The `.wasm` file could not be read or compiled.
    Load,
    /// Instantiation failed (bad component, missing import).
    Instantiate,
    /// The guest's run function trapped or returned failure.
    Run,
    /// A granted directory could not be preopened (missing on disk).
    Preopen,
}

/// Per-store state: WASI context + HTTP context + resource limits.
struct PluginState {
    ctx: WasiCtx,
    table: ResourceTable,
    http: WasiHttpCtx,
    limits: StoreLimits,
}

impl WasiView for PluginState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for PluginState {
    fn http(&mut self) -> wasmtime_wasi_http::p3::WasiHttpCtxView<'_> {
        wasmtime_wasi_http::p3::WasiHttpCtxView {
            hooks: &mut [],
            table: &mut self.table,
            ctx: &mut self.http,
        }
    }
}

/// Memory ceiling per plugin guest (256&nbsp;MiB).
const MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;

/// Builds the engine: epoch interruption on, component model on.
fn build_engine() -> Result<Engine, Report<EngineError>> {
    let mut config = Config::new();
    config.epoch_interruption(true);
    config.wasm_component_model(true);
    config.concurrency_support(true);
    Engine::new(&config).map_err(|e| Report::new(EngineError::Instantiate).attach(e.to_string()))
}

/// Instantiates and runs one plugin component under `grants`.
///
/// Drives the epoch deadline on a ticker so a runaway guest is preempted
/// even inside a tight loop. Returns when the guest's run function returns.
///
/// # Errors
///
/// Returns an error if the component fails to load, instantiate, or run.
pub async fn serve(wasm_path: &Path, grants: &Grants) -> Result<(), Report<EngineError>> {
    let engine = build_engine()?;

    let component = Component::from_file(&engine, wasm_path)
        .map_err(|e| Report::new(EngineError::Load).attach(format!("wasm load: {e}")))?;

    let mut linker: Linker<PluginState> = Linker::new(&engine);
    wasmtime_wasi::p3::add_to_linker(&mut linker)
        .map_err(|e| Report::new(EngineError::Instantiate).attach(format!("wasi link: {e}")))?;
    if grants.http {
        wasmtime_wasi_http::p3::add_to_linker(&mut linker).map_err(|e| {
            Report::new(EngineError::Instantiate).attach(format!("wasi:http link: {e}"))
        })?;
    }

    let store = build_store(&engine, grants)?;

    // Epoch ticker: every 10ms the epoch advances, preempting runaway guests
    // deterministically (the engine traps with "epoch deadline reached").
    let ticker = spawn_epoch_ticker(&engine);
    let result = run_guest(store, component, linker).await;
    ticker.abort();

    result
}

async fn run_guest(
    mut store: Store<PluginState>,
    component: Component,
    linker: Linker<PluginState>,
) -> Result<(), Report<EngineError>> {
    let command = Command::instantiate_async(&mut store, &component, &linker)
        .await
        .map_err(|e| Report::new(EngineError::Instantiate).attach(format!("instantiate: {e}")))?;

    let run = store
        .run_concurrent(async move |store| command.wasi_cli_run().call_run(store).await)
        .await
        .map_err(|e| Report::new(EngineError::Run).attach(format!("guest trap: {e}")))?
        .map_err(|e| Report::new(EngineError::Run).attach(format!("guest trap: {e}")))?;
    match run {
        Ok(()) => Ok(()),
        Err(()) => Err(Report::new(EngineError::Run)).attach("guest returned failure"),
    }
}

/// Builds the store with WASI context from the grants.
fn build_store(
    engine: &Engine,
    grants: &Grants,
) -> Result<Store<PluginState>, Report<EngineError>> {
    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdout().inherit_stderr().inherit_stdin();

    preopen_dirs(&mut builder, grants)?;

    let state = PluginState {
        ctx: builder.build(),
        table: ResourceTable::new(),
        http: WasiHttpCtx::new(),
        limits: StoreLimitsBuilder::new()
            .memory_size(MEMORY_LIMIT_BYTES)
            .build(),
    };
    let mut store = Store::new(engine, state);
    store.limiter(|state| &mut state.limits);
    store.set_epoch_deadline(1);
    Ok(store)
}

/// Preopens the granted directories: read grants as read-only, write
/// grants (including the default scratch dir) as read-write.
fn preopen_dirs(builder: &mut WasiCtxBuilder, grants: &Grants) -> Result<(), Report<EngineError>> {
    for dir in &grants.read_dirs {
        builder
            .preopened_dir(
                dir,
                dir.to_string_lossy(),
                wasmtime_wasi::DirPerms::READ,
                wasmtime_wasi::FilePerms::READ,
            )
            .map_err(|e| {
                Report::new(EngineError::Preopen)
                    .attach(format!("preopen read {}: {e}", dir.display()))
            })?;
    }
    for dir in &grants.write_dirs {
        if grants.read_dirs.contains(dir) {
            continue; // already preopened read-only; skip duplicate mount
        }
        std::fs::create_dir_all(dir).map_err(|e| {
            Report::new(EngineError::Preopen)
                .attach(format!("creating scratch dir {}: {e}", dir.display()))
        })?;
        builder
            .preopened_dir(
                dir,
                dir.to_string_lossy(),
                wasmtime_wasi::DirPerms::all(),
                wasmtime_wasi::FilePerms::all(),
            )
            .map_err(|e| {
                Report::new(EngineError::Preopen)
                    .attach(format!("preopen write {}: {e}", dir.display()))
            })?;
    }
    Ok(())
}

/// Spawns the background epoch ticker driving preemption.
fn spawn_epoch_ticker(engine: &Engine) -> tokio::task::JoinHandle<()> {
    let engine = engine.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(10));
        loop {
            interval.tick().await;
            engine.increment_epoch();
        }
    })
}
