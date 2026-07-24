//! Bus messages for MCP server lifecycle.
//!
//! The [`McpLifecycleActor`](crate::feat::mcp_lifecycle_actor) subscribes to
//! session lifecycle events (`SessionCreated`, `SessionLoadCompleted`,
//! `SessionClosed`, `SessionArchived`, `SessionTeardownFinished`) and to
//! [`McpEnablementChanged`] to spawn and kill `McpActor`s.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;

/// The set of MCP servers enabled for a session changed.
///
/// Published by the MCP picker confirm handler after writing the new
/// [`ChatSessionState::enabled_mcp_servers`](`crate::feat::session::chat_session::ChatSessionState::enabled_mcp_servers`)
/// set. Carries the **full** desired set (not a delta): the
/// [`McpLifecycleActor`](crate::feat::mcp_lifecycle_actor) diffs this against
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
