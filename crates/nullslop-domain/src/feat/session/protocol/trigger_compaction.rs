//! Command for manually triggering context compaction.

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;
use crate::protocol::CommandMsg;

/// Trigger compaction for a session.
///
/// Sent by the `/compact` or `/compact-all` slash commands.
/// The compaction trigger actor receives this command,
/// runs the compaction worker, and submits mutations.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("trigger_compaction")]
pub struct TriggerCompaction {
    /// The session to compact.
    pub session_id: SessionId,
    /// Whether to force-compact all entries (ignore reserve).
    pub compact_all: bool,
}
