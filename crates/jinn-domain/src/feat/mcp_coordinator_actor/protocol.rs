//! Bus messages for MCP server lifecycle.
//!
//! The [`McpCoordinatorActor`](crate::feat::mcp_coordinator_actor) subscribes to
//! session lifecycle events (`SessionCreated`, `SessionLoadCompleted`,
//! `SessionClosed`, `SessionArchived`, `SessionTeardownFinished`) and to
//! [`McpEnablementChanged`] to spawn and kill `McpActor`s.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;

/// Restart one (session × server) `McpActor`.
///
/// Sent to the [`McpCoordinatorActor`](crate::feat::mcp_coordinator_actor)
/// (e.g. by a future dashboard "restart" button). It kills the currently
/// spawned actor for the pair — if any — and respawns a fresh one, so a
/// wedged server process can be recovered without a full enable/disable
/// toggle through the picker. The respawn only proceeds if the server is
/// still present in the session's `enabled_mcp_servers` set; otherwise it's
/// a no-op.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartMcpServer {
    /// The session whose actor should restart.
    pub session_id: SessionId,
    /// The configured server name to restart.
    pub server: String,
}

impl crate::common::bus::BusMessage for RestartMcpServer {}

/// The set of MCP servers enabled for a session changed.
///
/// Published by the MCP picker confirm handler after writing the new
/// [`ChatSessionState::enabled_mcp_servers`](`crate::feat::session::chat_session::ChatSessionState::enabled_mcp_servers`)
/// set. Carries the **full** desired set (not a delta): the
/// [`McpCoordinatorActor`](crate::feat::mcp_coordinator_actor) diffs this against
/// its spawned-actor map, spawning newly-enabled servers and killing
/// newly-disabled ones.
///
/// This is per-session — each session maintains its own enablement, and each
/// enabled (session × server) pair owns an independent `McpActor` + child
/// process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpEnablementChanged {
    /// The session whose enablement set changed.
    pub session_id: SessionId,
    /// The full desired set of enabled server names after the change.
    pub enabled: BTreeSet<String>,
}

impl crate::common::bus::BusMessage for McpEnablementChanged {}
