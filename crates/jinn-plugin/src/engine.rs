//! WASM engine — one shared engine, one store per plugin guest.
//!
//! [`PluginEngine`] owns the process-wide wasmtime [`Engine`] (built once,
//! reused for every plugin: module compilation cost is paid per `.wasm`,
//! engine construction once). Each plugin guest gets its own [`Store`]
//! built from its granted [`Grants`] — preopened directories, optional
//! `wasi:http` — with epoch interruption (deterministic preemption of
//! runaway CPU) and store memory/table limits.
//!
//! The guest's stdio is wired to explicit in-memory sinks supplied by the
//! caller ([`PluginHost`](crate::PluginHost) passes the duplex pipe pair
//! that is the protocol channel; stderr drains to the shared
//! [`StderrRing`](crate::StderrRing)). A Rust guest using the SDK reads
//! host envelopes from stdin and writes plugin messages to stdout
//! directly. This module never interprets wire semantics; it only
//! executes with the right capabilities.

use std::path::Path;
use std::sync::Arc;

use error_stack::{Report, ResultExt as _};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p3::bindings::Command;
use wasmtime_wasi::cli::{AsyncStdinStream, AsyncStdoutStream};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::WasiHttpCtx;
use wasmtime_wasi_http::p3::WasiHttpView;

use crate::grants::Grants;
use crate::stderr_ring::StderrRing;

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

/// The process-wide wasmtime engine.
///
/// Clone-cheap (internally an `Arc`): every plugin shares it; each guest
/// runs in its own [`Store`] on its own spawned task.
#[derive(Clone)]
pub struct PluginEngine {
    engine: Engine,
}

impl std::fmt::Debug for PluginEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PluginEngine")
    }
}

impl PluginEngine {
    /// Builds the engine: epoch interruption on, component model on.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasmtime engine cannot be constructed.
    pub fn new() -> Result<Self, Report<EngineError>> {
        let mut config = Config::new();
        config.epoch_interruption(true);
        config.wasm_component_model(true);
        config.concurrency_support(true);
        let engine = Engine::new(&config)
            .map_err(|e| Report::new(EngineError::Instantiate).attach(e.to_string()))?;
        Ok(Self { engine })
    }

    /// Loads a `.wasm` file as a component (compiled once per module).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or is not a valid component.
    pub fn load(&self, wasm_path: &Path) -> Result<Component, Report<EngineError>> {
        Component::from_file(&self.engine, wasm_path)
            .map_err(|e| Report::new(EngineError::Load).attach(format!("wasm load: {e}")))
    }

    /// Instantiates and starts one guest on a spawned task under `grants`.
    ///
    /// `stdin`/`stdout` are the guest's WASI stdio (the in-memory protocol
    /// pipes); `stderr_ring` receives the guest's stderr. The returned
    /// join handle resolves when the guest's run function returns (or
    /// traps — reported as an `Err`). The epoch ticker runs for the
    /// lifetime of the task.
    ///
    /// # Errors
    ///
    /// Returns an error if the module cannot be loaded or instantiated.
    pub fn run_guest<R, W>(
        &self,
        wasm_path: &Path,
        grants: &Grants,
        stdin: R,
        stdout: W,
        stderr_ring: Arc<Mutex<StderrRing>>,
    ) -> Result<tokio::task::JoinHandle<Result<(), Report<EngineError>>>, Report<EngineError>>
    where
        R: AsyncRead + Send + Sync + 'static,
        W: AsyncWrite + Send + Sync + 'static,
    {
        let component = self.load(wasm_path)?;
        let store = build_store(&self.engine, grants, stdin, stdout, stderr_ring)?;

        let mut linker: Linker<PluginState> = Linker::new(&self.engine);
        wasmtime_wasi::p3::add_to_linker(&mut linker)
            .map_err(|e| Report::new(EngineError::Instantiate).attach(format!("wasi link: {e}")))?;
        if grants.http {
            wasmtime_wasi_http::p3::add_to_linker(&mut linker).map_err(|e| {
                Report::new(EngineError::Instantiate).attach(format!("wasi:http link: {e}"))
            })?;
        }

        // Epoch ticker: every 10ms the epoch advances, preempting runaway
        // guests deterministically (the engine traps with "epoch deadline
        // reached"). It lives only for this guest's task.
        let ticker = spawn_epoch_ticker(&self.engine);
        let task = tokio::spawn(async move {
            let result = drive_guest(store, component, linker).await;
            ticker.abort();
            result
        });
        Ok(task)
    }
}

/// Instantiates and drives one guest to completion.
async fn drive_guest(
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

/// Builds the store with WASI context from the grants and the explicit
/// stdio sinks.
fn build_store<R, W>(
    engine: &Engine,
    grants: &Grants,
    stdin: R,
    stdout: W,
    stderr_ring: Arc<Mutex<StderrRing>>,
) -> Result<Store<PluginState>, Report<EngineError>>
where
    R: AsyncRead + Send + Sync + 'static,
    W: AsyncWrite + Send + Sync + 'static,
{
    let stderr = StderrToRing::new(stderr_ring);
    let mut builder = WasiCtxBuilder::new();
    builder
        .stdin(AsyncStdinStream::new(stdin))
        .stdout(AsyncStdoutStream::new(64 * 1024, stdout))
        .stderr(AsyncStdoutStream::new(16 * 1024, stderr));
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
            Report::new(EngineError::Preopen).attach(format!("creating scratch dir {}: {e}", dir.display()))
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

/// A [`tokio::io::AsyncWrite`] that appends lines into the shared
/// [`StderrRing`], so guest diagnostics never reach jinn's terminal.
struct StderrToRing {
    ring: Arc<Mutex<StderrRing>>,
    pending: Vec<u8>,
}

impl StderrToRing {
    fn new(ring: Arc<Mutex<StderrRing>>) -> Self {
        Self {
            ring,
            pending: Vec::new(),
        }
    }
}

impl AsyncWrite for StderrToRing {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.pending.extend_from_slice(buf);
        // Extract complete lines; the remainder waits for more bytes.
        while let Some(pos) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=pos).collect();
            let trimmed = line.get(..line.len().saturating_sub(1)).unwrap_or(&line);
            let line = String::from_utf8_lossy(trimmed).into_owned();
            if let Ok(mut ring) = self.ring.try_lock() {
                ring.append_line(&line);
            }
        }
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

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
