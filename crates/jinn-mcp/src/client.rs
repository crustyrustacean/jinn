//! MCP client connection to a single server over a stdio child process.
//!
//! This module is transport-level: [`McpClient`] owns one `rmcp` client
//! connection to one MCP server process. It knows nothing about the actor
//! system or `AppState`. The `jinn-domain::feat::mcp_actor` module drives it.
//!
//! # Lifecycle
//!
//! [`McpClient::connect`] spawns the server process (with `kill_on_drop` so a
//! dropped connection always terminates the child), performs the MCP
//! initialize handshake, and is ready to list/call tools. [`McpClient::shutdown`]
//! closes the transport and waits (with a timeout) for the child to exit.

use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use error_stack::{Report, ResultExt};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::child_process::TokioChildProcess;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use wherror::Error;

/// A server process failed to start or communicate.
#[derive(Debug, Error)]
#[error(debug)]
pub struct McpClientError;

/// The command line used to spawn an MCP server.
///
/// A thin owned value so callers (the actor system) can carry it cheaply
/// without importing `rmcp`/`tokio` types.
#[derive(Debug, Clone)]
pub struct ServerCommand {
    /// The executable to run (e.g. `"npx"`).
    pub program: String,
    /// Arguments passed to the executable.
    pub args: Vec<String>,
}

/// A live connection to one MCP server.
///
/// Owns the `rmcp` [`RunningService`], which in turn owns the child process.
/// Dropping this value cancels the transport and (via `kill_on_drop` on the
/// child) terminates the server process asynchronously; prefer
/// [`McpClient::shutdown`] for bounded, deterministic cleanup.
///
/// The child's **stderr** is piped (never inherited) and drained by a
/// background task into a bounded [`McpStderrBuffer`], so server diagnostics
/// never corrupt jinn's terminal.
pub struct McpClient {
    service: RunningService<RoleClient, ()>,
    /// Shared stderr ring buffer, written by the drain task.
    stderr_buffer: Arc<Mutex<McpStderrBuffer>>,
}

impl McpClient {
    /// Spawns the server process and completes the MCP initialize handshake.
    ///
    /// The child's stderr is piped and drained into an internal ring buffer
    /// (see [`McpClient::stderr_tail`]); it never reaches jinn's terminal.
    ///
    /// # Errors
    ///
    /// Returns an error if the process cannot be spawned or the handshake fails.
    pub async fn connect(cmd: &ServerCommand) -> Result<Self, Report<McpClientError>> {
        let mut command = Command::new(&cmd.program);
        command.args(&cmd.args);
        command.kill_on_drop(true);

        let (transport, stderr) = TokioChildProcess::builder(command)
            .stderr(Stdio::piped())
            .spawn()
            .change_context(McpClientError)
            .attach("failed to spawn MCP server process")?;

        let stderr_buffer = Arc::new(Mutex::new(McpStderrBuffer::new()));
        spawn_stderr_drain(stderr, stderr_buffer.clone());

        let service =
            ().serve(transport)
                .await
                .change_context(McpClientError)
                .attach("MCP initialize handshake failed")?;

        Ok(Self {
            service,
            stderr_buffer,
        })
    }

    /// Connects over an arbitrary transport instead of spawning a child process.
    ///
    /// Used by integration tests that drive an in-process stub MCP server
    /// over an in-memory duplex pipe. Gated behind the `testkit` feature so it
    /// can never appear in production code paths. There is no child process, so
    /// the stderr buffer is empty.
    ///
    /// # Errors
    ///
    /// Returns an error if the MCP initialize handshake fails over the transport.
    #[cfg(feature = "testkit")]
    pub async fn connect_with_transport<T, E, A>(
        transport: T,
    ) -> Result<Self, Report<McpClientError>>
    where
        T: rmcp::transport::IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let service =
            ().serve(transport)
                .await
                .change_context(McpClientError)
                .attach("MCP initialize handshake failed")?;
        Ok(Self {
            service,
            stderr_buffer: Arc::new(Mutex::new(McpStderrBuffer::new())),
        })
    }

    /// Lists the tools the server exposes.
    ///
    /// # Errors
    ///
    /// Returns an error if the `tools/list` request fails.
    pub async fn list_tools(&self) -> Result<Vec<rmcp::model::Tool>, Report<McpClientError>> {
        self.service
            .list_tools(Option::default())
            .await
            .map(|result| result.tools)
            .change_context(McpClientError)
            .attach("tools/list request failed")
    }

    /// Calls a tool by name with optional JSON object arguments.
    ///
    /// # Errors
    ///
    /// Returns an error if the `tools/call` request fails at the transport or
    /// protocol level. A tool-reported error (`is_error`) is returned as a
    /// successful [`CallToolResult`], not an [`Err`].
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<rmcp::model::JsonObject>,
    ) -> Result<CallToolResult, Report<McpClientError>> {
        let mut params = CallToolRequestParams::new(name.to_owned());
        if let Some(args) = arguments {
            params = params.with_arguments(args);
        }
        self.service
            .call_tool(params)
            .await
            .change_context(McpClientError)
            .attach("tools/call request failed")
    }

    /// Returns a snapshot of the captured stderr tail (newest content).
    ///
    /// May be empty if the server hasn't written anything to stderr.
    #[must_use]
    pub fn stderr_tail(&self) -> String {
        self.stderr_buffer
            .lock()
            .map(|buf| buf.tail().to_owned())
            .unwrap_or_default()
    }

    /// Gracefully closes the connection and waits (bounded) for the child to exit.
    pub async fn shutdown(&mut self) {
        // close_with_timeout cancels the transport; kill_on_drop guarantees
        // the child dies if it doesn't exit on its own.
        if let Err(e) = self
            .service
            .close_with_timeout(Duration::from_secs(5))
            .await
        {
            tracing::warn!(error = ?e, "MCP client shutdown join error");
        }
    }
}

// ─── stderr ring buffer ────────────────────────────────────────────────

/// Maximum bytes of child stderr to retain (the most recent tail).
const MCP_STDERR_MAX_BYTES: usize = 16 * 1024;

/// Bounded ring buffer for captured child-process stderr.
///
/// Keeps the most recent lines (by byte budget). Older content is dropped
/// when the budget is exceeded, mirroring the `bash` tool's streaming
/// truncation: newest content survives, memory is bounded.
#[derive(Debug, Clone)]
pub struct McpStderrBuffer {
    content: String,
    max_bytes: usize,
}

impl Default for McpStderrBuffer {
    fn default() -> Self {
        Self::new()
    }
}
impl McpStderrBuffer {
    /// Creates a buffer with the default 16&nbsp;KB tail budget.
    #[must_use]
    pub fn new() -> Self {
        Self::with_budget(MCP_STDERR_MAX_BYTES)
    }

    /// Creates a buffer with a custom byte budget.
    #[must_use]
    pub fn with_budget(max_bytes: usize) -> Self {
        Self {
            content: String::new(),
            max_bytes,
        }
    }

    /// Appends one line and trims to the byte budget (keeping the tail).
    pub fn append_line(&mut self, line: &str) {
        if !self.content.is_empty() {
            self.content.push('\n');
        }
        self.content.push_str(line);
        self.trim_to_budget();
    }

    /// Drops leading content until the buffer fits within `max_bytes`,
    /// advancing to the next UTF-8 char boundary so multibyte characters are
    /// never split.
    fn trim_to_budget(&mut self) {
        if self.content.len() <= self.max_bytes {
            return;
        }
        let cut = self.content.len().saturating_sub(self.max_bytes);
        let mut start = cut;
        while !self.content.is_char_boundary(start) {
            start += 1;
        }
        self.content.drain(0..start);
    }

    /// Returns the captured tail (may be empty).
    #[must_use]
    pub fn tail(&self) -> &str {
        &self.content
    }
}

/// Spawns a background task that drains `stderr` line-by-line into the shared
/// buffer. The task exits naturally when the child process dies and the pipe
/// closes (EOF), so no explicit cancellation is needed — `kill_on_drop`
/// guarantees the child terminates.
fn spawn_stderr_drain(
    stderr: Option<tokio::process::ChildStderr>,
    buffer: Arc<Mutex<McpStderrBuffer>>,
) {
    let Some(stderr) = stderr else {
        return;
    };
    tokio::spawn(async move {
        let reader = tokio::io::BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(mut buf) = buffer.lock() {
                buf.append_line(&line);
            }
        }
        // EOF or error: child died or pipe closed — drain exits.
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_buffer_starts_empty() {
        // Given a fresh buffer.
        let buf = McpStderrBuffer::default();

        // Then its tail is empty.
        assert!(buf.tail().is_empty());
    }

    #[test]
    fn stderr_buffer_appends_and_joins_lines() {
        // Given a fresh buffer.
        let mut buf = McpStderrBuffer::default();

        // When appending two lines.
        buf.append_line("first");
        buf.append_line("second");

        // Then both are retained, newline-joined.
        assert_eq!(buf.tail(), "first\nsecond");
    }

    #[test]
    fn stderr_buffer_drops_oldest_when_over_byte_budget() {
        // Given a buffer with a tiny 10-byte budget.
        let mut buf = McpStderrBuffer::with_budget(10);

        // When appending lines that overflow the budget.
        buf.append_line("AAAAA"); // 5 bytes, fits
        buf.append_line("BBBBB"); // 5 + 1 newline + 5 = 11, trims oldest

        // Then only the most recent content survives and the budget holds.
        assert!(buf.tail().len() <= 11);
        assert!(buf.tail().ends_with("BBBBB"));
        assert!(!buf.tail().starts_with("AAAAA"));
    }
}
