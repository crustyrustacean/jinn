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

use std::time::Duration;

use error_stack::{Report, ResultExt};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::child_process::TokioChildProcess;
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
pub struct McpClient {
    service: RunningService<RoleClient, ()>,
}

impl McpClient {
    /// Spawns the server process and completes the MCP initialize handshake.
    ///
    /// # Errors
    ///
    /// Returns an error if the process cannot be spawned or the handshake fails.
    pub async fn connect(cmd: &ServerCommand) -> Result<Self, Report<McpClientError>> {
        let mut command = Command::new(&cmd.program);
        command.args(&cmd.args);
        command.kill_on_drop(true);
        let transport = TokioChildProcess::new(command)
            .change_context(McpClientError)
            .attach("failed to spawn MCP server process")?;

        let service =
            ().serve(transport)
                .await
                .change_context(McpClientError)
                .attach("MCP initialize handshake failed")?;

        Ok(Self { service })
    }

    /// Connects over an arbitrary transport instead of spawning a child process.
    ///
    /// Used by integration tests that drive an in-process stub MCP server
    /// over an in-memory duplex pipe. Gated behind the `testkit` feature so it
    /// can never appear in production code paths.
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
        Ok(Self { service })
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
