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
use std::time::Instant;

use error_stack::{Report, ResultExt as _};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use wasmtime::component::{Component, Linker};
use wasmtime::{Cache, CacheConfig, Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::cli::{AsyncStdinStream, AsyncStdoutStream};
use wasmtime_wasi::p2::bindings::Command;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::WasiHttpCtx;
use wasmtime_wasi_http::p2::{WasiHttpCtxView, WasiHttpView};

use crate::grants::Grants;
use crate::stderr_ring::StderrRing;

/// How one [`PluginEngine::load`] obtained its component.
#[derive(Debug, Clone, Copy)]
pub struct Compiled {
    /// `true` when the compiled artifact came from the disk cache (no
    /// compilation happened); `false` when the wasm was freshly JIT-compiled.
    pub from_cache: bool,
    /// Wall-clock time the load took — compile time on a miss, deserialize
    /// time on a hit.
    pub duration: std::time::Duration,
}

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
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
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
    cache: Option<Cache>,
    /// Serializes component compilation. wasmtime's hit/miss counters are
    /// process-global, so concurrent compiles would attribute each other's
    /// misses to the wrong plugin; a gate gives exact per-plugin reporting
    /// and avoids oversubscribing cores when several plugins JIT at once.
    compile_gate: Arc<tokio::sync::Mutex<()>>,
}

impl std::fmt::Debug for PluginEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PluginEngine")
    }
}

impl PluginEngine {
    /// Builds the engine: epoch interruption on, component model on, disk
    /// cache on.
    ///
    /// The cache persists compiled component artifacts under the OS cache dir
    /// (`~/.cache/wasmtime/…`), so a plugin compiled once loads from disk on
    /// every later launch instead of re-JITing (~seconds → ~milliseconds). A
    /// cache failure is fatal at engine construction (directory cannot be
    /// created); once constructed, cache write failures degrade silently to
    /// compile-per-launch.
    ///
    /// # Errors
    ///
    /// Returns an error if the wasmtime engine cannot be constructed or the
    /// cache cannot be configured.
    pub fn new() -> Result<Self, Report<EngineError>> {
        let cache = {
            let cache_config = CacheConfig::new();
            let cache = Cache::new(cache_config)
                .map_err(|e| Report::new(EngineError::Instantiate).attach(format!("cache: {e}")))?;
            Some(cache)
        };
        let mut config = Config::new();
        config.epoch_interruption(true);
        config.wasm_component_model(true);
        config.concurrency_support(true);
        config.cache(cache.clone());
        let engine = Engine::new(&config)
            .map_err(|e| Report::new(EngineError::Instantiate).attach(e.to_string()))?;
        Ok(Self {
            engine,
            cache,
            compile_gate: Arc::new(tokio::sync::Mutex::const_new(())),
        })
    }

    /// Loads a `.wasm` file as a component (compiled once per module).
    ///
    /// Returns how the component was obtained so callers can report
    /// progress only when real compilation happened.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or is not a valid component.
    pub async fn load(
        &self,
        wasm_path: &Path,
    ) -> Result<(Component, Compiled), Report<EngineError>> {
        // Gate around the whole hit-or-miss decision: wasmtime counts hits
        // and misses on process-global counters, so without the gate two
        // plugins compiling concurrently could each observe the other's
        // delta.
        let gate = self.compile_gate.lock().await;
        let started = Instant::now();
        let (hits_before, misses_before) = self.cache_counters();
        let component = Component::from_file(&self.engine, wasm_path)
            .map_err(|e| Report::new(EngineError::Load).attach(format!("wasm load: {e}")))?;
        let (hits_after, misses_after) = self.cache_counters();
        drop(gate);

        let duration = started.elapsed();
        let from_cache = match self.cache.as_ref() {
            // The disk cache answered: either the hit counter advanced, or
            // no new miss was recorded — the artifact was found and
            // deserialized rather than compiled.
            Some(_) => hits_after > hits_before || misses_after == misses_before,
            // No cache configured: everything is compiled fresh.
            None => false,
        };
        let report = Compiled {
            from_cache,
            duration,
        };
        Ok((component, report))
    }

    /// Snapshot of the engine cache's global hit/miss counters.
    fn cache_counters(&self) -> (usize, usize) {
        match self.cache.as_ref() {
            Some(cache) => (cache.cache_hits(), cache.cache_misses()),
            None => (0, 0),
        }
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
    pub async fn run_guest<R, W>(
        &self,
        wasm_path: &Path,
        grants: &Grants,
        stdin: R,
        stdout: W,
        stderr_ring: Arc<Mutex<StderrRing>>,
    ) -> Result<
        (
            tokio::task::JoinHandle<Result<(), Report<EngineError>>>,
            Compiled,
        ),
        Report<EngineError>,
    >
    where
        R: AsyncRead + Send + Sync + 'static,
        W: AsyncWrite + Send + Sync + 'static,
    {
        let (component, report) = self.load(wasm_path).await?;
        let store = build_store(&self.engine, grants, stdin, stdout, stderr_ring)?;

        let mut linker: Linker<PluginState> = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| Report::new(EngineError::Instantiate).attach(format!("wasi link: {e}")))?;
        if grants.http {
            wasmtime_wasi_http::p2::add_to_linker_async(&mut linker).map_err(|e| {
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
        Ok((task, report))
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

    let run = command
        .wasi_cli_run()
        .call_run(&mut store)
        .await
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
    // Async execution + epoch interruption: the deadline callback yields
    // to the tokio runtime and refreshes the deadline instead of trapping,
    // so a long-lived guest is preempted cooperatively rather than killed
    // on the first epoch tick. `shutdown` still aborts hard.
    store.epoch_deadline_async_yield_and_update(1);
    Ok(store)
}

/// Preopens the granted directories: read grants as read-only, write
/// grants as read-write. Writable preopens are created on demand: a
/// preopen requires the path to exist.
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
