//! In-process plugin host — instantiate a wasm guest on a dedicated task.
//!
//! One [`PluginHost`] per plugin: it builds a wasmtime [`Store`] from the
//! granted [`Grants`] (preopened dirs, optional `wasi:http`), wires the
//! guest's stdio to in-memory pipes (a [`tokio::io::duplex`] pair — the
//! host side is the protocol channel, the guest side is mounted as the
//! guest's WASI stdin/stdout), runs the guest's `wasi:cli/command` run
//! function on a spawned task, and drains guest stderr into a bounded
//! [`StderrRing`].
//!
//! Dropping this value aborts the guest task. The wasm sandbox is the
//! isolation boundary; the epoch interruption caps CPU; store limits cap
//! memory. There is no child process, no pid, no kill tree.
//!
//! This layer moves bytes and frames lines — trust decisions live
//! **upstream** in the plugin coordinator (jinn-domain).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use error_stack::{Report, ResultExt as _};
use jinn_plugin_api::Envelope;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::engine::PluginEngine;
use crate::framing::{decode_envelope, encode_envelope};
use crate::grants::Grants;
use crate::stderr_ring::StderrRing;

/// A plugin host failed.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum PluginHostError {
    /// The `.wasm` module could not be loaded or instantiated.
    Start,
    /// Writing to the guest failed (it ended or wedged).
    Write,
    /// Reading from the guest failed.
    Read,
}

/// Facts about one hosted plugin.
#[derive(Debug, Clone)]
pub struct SpawnInfo {
    /// The plugin's manifest name.
    pub name: String,
    /// Path to the `.wasm` module being hosted.
    pub wasm_path: PathBuf,
}

/// Pipe capacity in bytes for the in-memory stdio channel (both directions).
///
/// One envelope per message; a full theme set is a few kilobytes, so 64&nbsp;KiB
/// is generous headroom while bounding buffered bytes per plugin.
const PIPE_CAPACITY_BYTES: usize = 64 * 1024;

/// A live plugin guest, hosted in-process.
///
/// The guest task runs to completion (or trap) on a spawned task; its
/// stdio is the [`Grants`]-independent protocol channel. [`PluginHost::write`]
/// sends one envelope; [`PluginHost::split`] hands the read half to a pump
/// task. Dropping this value aborts the guest (wasmtime stores are
/// `Send`; aborting the task mid-run drops the store and its resources).
pub struct PluginHost {
    /// The guest task — aborted on drop.
    guest_task: tokio::task::JoinHandle<Result<(), Report<crate::engine::EngineError>>>,
    write: HostWriter,
    /// The stdout read half, until [`PluginHost::split`] takes it.
    read_half: Option<BufReader<tokio::io::DuplexStream>>,
    /// Shared stderr ring, written by the drain task.
    stderr_ring: Arc<Mutex<StderrRing>>,
    spawn: SpawnInfo,
}

impl PluginHost {
    /// Instantiates and starts the guest for one plugin.
    ///
    /// `engine` is the shared wasmtime engine (one per process, reused
    /// across plugins — compile once, instantiate per plugin).
    ///
    /// # Errors
    ///
    /// Returns an error if the module cannot be loaded or instantiated.
    pub fn start(
        engine: &PluginEngine,
        name: &str,
        wasm_path: &Path,
        grants: &Grants,
    ) -> Result<Self, Report<PluginHostError>> {
        let (host_to_guest_tx, guest_rx) = tokio::io::duplex(PIPE_CAPACITY_BYTES);
        let (guest_tx, host_rx) = tokio::io::duplex(PIPE_CAPACITY_BYTES);
        let stderr_ring = Arc::new(Mutex::new(StderrRing::new()));

        let guest_task = engine
            .run_guest(wasm_path, grants, guest_rx, guest_tx, stderr_ring.clone())
            .change_context(PluginHostError::Start)
            .attach(format!("failed to start plugin guest {name}"))?;

        Ok(Self {
            guest_task,
            write: HostWriter {
                stdin: host_to_guest_tx,
            },
            read_half: Some(BufReader::new(host_rx)),
            stderr_ring,
            spawn: SpawnInfo {
                name: name.to_owned(),
                wasm_path: wasm_path.to_path_buf(),
            },
        })
    }

    /// Splits off the stdout read half for a pump task.
    ///
    /// The returned [`PluginReader`] owns the guest's stdout; this host
    /// keeps the writer, stderr ring, and guest task. A second call yields
    /// a reader over an ended pipe (the pump task ends immediately at EOF)
    /// rather than panicking — misuse degrades, it does not crash.
    #[must_use]
    pub fn split(&mut self) -> PluginReader {
        let (ended_tx, ended_rx) = tokio::io::duplex(1);
        drop(ended_tx); // no writer: immediate EOF
        let stdout = self.read_half.take().unwrap_or_else(|| BufReader::new(ended_rx));
        PluginReader {
            stdout,
            name: self.spawn.name.clone(),
        }
    }

    /// Facts about the hosted plugin (name, wasm path).
    #[must_use]
    pub fn spawn_info(&self) -> &SpawnInfo {
        &self.spawn
    }

    /// The most recent stderr tail from the guest.
    #[must_use]
    pub async fn stderr_tail(&self) -> String {
        self.stderr_ring.lock().await.tail().to_owned()
    }

    /// Whether the guest task has finished (returned, trapped, or aborted).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.guest_task.is_finished()
    }

    /// Writes one envelope to the guest's stdin (one NDJSON line).
    ///
    /// # Errors
    ///
    /// Returns an error if the guest side of the pipe has closed.
    pub async fn write(&mut self, envelope: &Envelope) -> Result<(), Report<PluginHostError>> {
        let line = encode_envelope(envelope)
            .change_context(PluginHostError::Write)
            .attach("failed to encode envelope")?;
        self.write
            .stdin
            .write_all(&line)
            .await
            .change_context(PluginHostError::Write)
            .attach("plugin stdin write failed")?;
        self.write
            .stdin
            .flush()
            .await
            .change_context(PluginHostError::Write)
            .attach("plugin stdin flush failed")?;
        Ok(())
    }

    /// Reads the next envelope from the guest's stdout.
    ///
    /// Returns `Ok(None)` on EOF (guest ended). Malformed lines are
    /// skipped with a warn log — hostile input never fails the read.
    ///
    /// # Errors
    ///
    /// Returns an error only on I/O failure of the pipe itself.
    pub async fn read(&mut self) -> Result<Option<Envelope>, Report<PluginHostError>> {
        let Some(stdout) = self.read_half.take() else {
            return Err(Report::new(PluginHostError::Read).attach("read half was split off"));
        };
        let mut reader = PluginReader {
            stdout,
            name: self.spawn.name.clone(),
        };
        let result = reader.read_next().await;
        self.read_half = Some(reader.stdout);
        result
    }

    /// Aborts the guest and waits for the task to end.
    ///
    /// Bounded: abort is cooperative at the next await point (or epoch
    /// tick inside wasm execution), so this always completes quickly for
    /// a sandboxed guest; a guest wedged inside a host import is still
    /// forcibly detached when the task is dropped.
    pub async fn shutdown(&mut self) {
        let task = std::mem::replace(&mut self.guest_task, tokio::spawn(std::future::pending()));
        task.abort();
        let _ = task.await;
        let _ = self.write.stdin.shutdown().await;
    }
}

/// The write half of the guest's stdio: the host→guest pipe.
struct HostWriter {
    stdin: tokio::io::DuplexStream,
}

/// The read half of the guest's stdout, for a pump task.
///
/// Obtained via [`PluginHost::split`]. Reading never fails on hostile
/// input — malformed lines are skipped with a warn log.
pub struct PluginReader {
    stdout: BufReader<tokio::io::DuplexStream>,
    /// The plugin name, for log context.
    name: String,
}

impl PluginReader {
    /// Reads the next valid envelope from the guest's stdout.
    ///
    /// Returns `Ok(None)` on EOF (guest ended). Malformed lines are
    /// skipped with a warn log — hostile input never fails the read.
    ///
    /// # Errors
    ///
    /// Returns an error only on I/O failure of the pipe itself.
    pub async fn read_next(&mut self) -> Result<Option<Envelope>, Report<PluginHostError>> {
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .change_context(PluginHostError::Read)
                .attach("plugin stdout read failed")?;
            if n == 0 {
                return Ok(None);
            }
            match decode_envelope(line.as_bytes()) {
                Ok(Some(envelope)) => return Ok(Some(envelope)),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(plugin = %self.name, err = ?e, "dropping malformed plugin line");
                }
            }
        }
    }
}
