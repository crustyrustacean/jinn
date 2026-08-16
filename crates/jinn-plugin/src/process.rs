//! The runner child process — spawn, NDJSON pump, bounded shutdown.
//!
//! The coordinator spawns jinn's own executable with
//! `--serve-wasm-plugin`; the child instantiates the WASM guest under the
//! granted capabilities and pipes its wire traffic over stdio. This type is
//! the host-side handle to that child: it writes host→plugin envelopes,
//! reads plugin→host lines, and owns the bounded stderr ring.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use error_stack::{Report, ResultExt as _};
use jinn_plugin_api::Envelope;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::framing::{decode_envelope, encode_envelope};
use crate::grants::Grants;
use crate::stderr_ring::StderrRing;

/// A plugin runner process failed.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct PluginProcessError;

/// Facts about one spawned plugin runner.
#[derive(Debug, Clone)]
pub struct SpawnInfo {
    /// The plugin's manifest name.
    pub name: String,
    /// Path to the `.wasm` module the child will instantiate.
    pub wasm_path: PathBuf,
    /// Child process id.
    pub pid: u32,
}

/// A live plugin runner child.
///
/// stdout is the protocol channel; stderr is drained to a bounded
/// [`StderrRing`] so guest diagnostics never reach jinn's terminal.
/// Dropping this value kills the child (`kill_on_drop`); prefer
/// [`PluginProcess::shutdown`] for bounded deterministic cleanup.
///
/// Ownership splits for actor use: [`PluginProcess::split`] hands the
/// stdout read half to a pump task ([`PluginReader`]) while the actor
/// keeps the writer half; the child stays owned here so kill-on-drop
/// still reaps the whole process when the actor dies.
pub struct PluginProcess {
    child: Child,
    write: PluginWriter,
    /// The stdout read half, until [`PluginProcess::split`] takes it.
    read_half: Option<BufReader<ChildStdout>>,
    /// Shared stderr ring, written by the drain task.
    stderr_ring: Arc<Mutex<StderrRing>>,
    spawn: SpawnInfo,
}

/// The write half of a runner child: stdin.
struct PluginWriter {
    stdin: ChildStdin,
}

/// The read half of a runner child's stdout, for a pump task.
///
/// Obtained via [`PluginProcess::split`]. Reading never fails on hostile
/// input — malformed lines are skipped with a warn log.
pub struct PluginReader {
    stdout: BufReader<ChildStdout>,
    /// The plugin name, for log context.
    name: String,
}

impl PluginReader {
    /// Reads the next valid envelope from the child's stdout.
    ///
    /// Returns `Ok(None)` on EOF (child exited). Malformed lines are
    /// skipped with a warn log — hostile input never fails the read.
    ///
    /// # Errors
    ///
    /// Returns an error only on I/O failure of the stdout pipe itself.
    pub async fn read_next(&mut self) -> Result<Option<Envelope>, Report<PluginProcessError>> {
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .change_context(PluginProcessError)
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

impl PluginProcess {
    /// Spawns the runner child from jinn's own executable.
    ///
    /// `exe` is jinn's binary (the coordinator passes
    /// `std::env::current_exe()`; tests pass a fake). Grants travel via
    /// environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if the child cannot be spawned.
    pub fn spawn(
        exe: &std::path::Path,
        name: &str,
        wasm_path: &std::path::Path,
        grants: &Grants,
    ) -> Result<Self, Report<PluginProcessError>> {
        let mut command = Command::new(exe);
        command
            .arg("--serve-wasm-plugin")
            .arg(wasm_path)
            .env("JINN_PLUGIN_ID", name)
            .env(
                "JINN_PROTOCOL_VERSION",
                jinn_plugin_api::PROTOCOL_VERSION.to_string(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        for (key, value) in grants.env_pairs() {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .change_context(PluginProcessError)
            .attach(format!("failed to spawn plugin runner for {name}"))?;

        let pid = child.id().unwrap_or(0);
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Report::new(PluginProcessError).attach("child stdin not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Report::new(PluginProcessError).attach("child stdout not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Report::new(PluginProcessError).attach("child stderr not piped"))?;

        let stderr_ring = Arc::new(Mutex::new(StderrRing::new()));
        spawn_stderr_drain(stderr, Arc::clone(&stderr_ring));

        Ok(Self {
            child,
            write: PluginWriter { stdin },
            read_half: Some(BufReader::new(stdout)),
            stderr_ring,
            spawn: SpawnInfo {
                name: name.to_owned(),
                wasm_path: wasm_path.to_path_buf(),
                pid,
            },
        })
    }

    /// Splits off the stdout read half for a pump task.
    ///
    /// The returned [`PluginReader`] owns the child's stdout; this process
    /// keeps the child, stdin, and stderr ring. Panics if called twice.
    #[must_use]
    pub fn split(&mut self) -> PluginReader {
        PluginReader {
            stdout: self.read_half.take().expect("split called twice"),
            name: self.spawn.name.clone(),
        }
    }

    /// Facts about the spawn (name, wasm path, pid).
    #[must_use]
    pub fn spawn_info(&self) -> &SpawnInfo {
        &self.spawn
    }

    /// The most recent stderr tail from the guest.
    #[must_use]
    pub async fn stderr_tail(&self) -> String {
        self.stderr_ring.lock().await.tail().to_owned()
    }

    /// Writes one envelope to the child's stdin (one NDJSON line).
    ///
    /// # Errors
    ///
    /// Returns an error if the child's stdin has closed (process died).
    pub async fn write(&mut self, envelope: &Envelope) -> Result<(), Report<PluginProcessError>> {
        let line = encode_envelope(envelope)
            .change_context(PluginProcessError)
            .attach("failed to encode envelope")?;
        self.write
            .stdin
            .write_all(&line)
            .await
            .change_context(PluginProcessError)
            .attach("plugin stdin write failed")?;
        self.write
            .stdin
            .flush()
            .await
            .change_context(PluginProcessError)
            .attach("plugin stdin flush failed")?;
        Ok(())
    }

    /// Reads the next envelope from the child's stdout.
    ///
    /// Returns `Ok(None)` on EOF (child exited). Malformed lines are
    /// skipped internally with a warn log — reading never fails on hostile
    /// input, it just yields the next good line (or `None` at EOF).
    ///
    /// # Errors
    ///
    /// Returns an error only on I/O failure of the stdout pipe itself.
    pub async fn read(&mut self) -> Result<Option<Envelope>, Report<PluginProcessError>> {
        let Some(stdout) = self.read_half.take() else {
            return Err(Report::new(PluginProcessError).attach("read half was split off"));
        };
        let mut reader = PluginReader {
            stdout,
            name: self.spawn.name.clone(),
        };
        let result = reader.read_next().await;
        self.read_half = Some(reader.stdout);
        result
    }

    /// Sends a graceful stop and waits bounded for exit; force-kills on
    /// timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if the wait itself fails.
    pub async fn shutdown(&mut self) -> Result<(), Report<PluginProcessError>> {
        let _ = self.write.stdin.shutdown().await;
        match tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await {
            Ok(_) => Ok(()),
            Err(_) => {
                let _ = self.child.start_kill();
                Ok(())
            }
        }
    }
}

/// Drains the child's stderr into the shared ring until EOF.
fn spawn_stderr_drain(stderr: ChildStderr, ring: Arc<Mutex<StderrRing>>) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            ring.lock().await.append_line(&line);
        }
    });
}
