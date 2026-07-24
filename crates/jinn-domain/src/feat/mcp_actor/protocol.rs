//! Bus protocol for `McpActor` status reporting.
//!
//! [`McpActor`](crate::feat::mcp_actor::McpActor) publishes [`McpServerStatus`]
//! as its connection transitions through Starting → Running → Dead. The
//! dashboard tab (a follow-up task) subscribes to render per-(session × server)
//! process state. No UI is built in this phase — the event exists so the data
//! is available.
//!
//! A dead actor is _not_ auto-restarted (the supervisor policy is
//! [`RestartPolicy::Never`]). The user re-enables the server, or a future
//! dashboard "restart" capability calls [`RestartMcpServer`]
//! (defined in [`crate::feat::mcp_lifecycle_actor::protocol`]).

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;

/// Coarse connection state of one `McpActor`'s child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpConnectionStatus {
    /// `on_start` is running: the child process is being spawned / `initialize`
    /// and `tools/list` are in flight.
    Starting,
    /// `tools/list` succeeded; tools are registered and the actor answers
    /// `ExecuteTool` calls.
    Running,
    /// The connection never came up (spawn/initialize/list failed) or has been
    /// shut down. The actor may still be alive but idle; tool calls for this
    /// server return a failed result.
    Dead,
}

/// A connection-status transition for one (session × server) `McpActor`.
///
/// Published by `McpActor` at every transition. Subscribers can build a live
/// view of every MCP process in the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerStatus {
    /// The session the actor serves.
    pub session_id: SessionId,
    /// The configured server name.
    pub server: String,
    /// The new connection state.
    pub status: McpConnectionStatus,
}

impl crate::common::bus::BusMessage for McpServerStatus {}

/// Captured stderr tail for one (session × server) `McpActor`.
///
/// Published by `McpActor` whenever new child-process stderr is drained.
/// The payload is the bounded tail (newest content); subscribers keep a live
/// view for a future log viewer. Published best-effort, alongside status
/// transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerLog {
    /// The session the actor serves.
    pub session_id: SessionId,
    /// The configured server name.
    pub server: String,
    /// The newest captured stderr content (bounded).
    pub tail: String,
}

impl crate::common::bus::BusMessage for McpServerLog {}
