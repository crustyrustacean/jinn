//! Compaction command types.

use serde::{Deserialize, Serialize};

use crate::protocol::{CommandMsg, SessionId};

/// Request to compact the conversation context for a session.
///
/// The `CompactionActor` receives this command, gathers entries from the
/// last compaction boundary to the cut point, summarizes them via an LLM,
/// marks gathered entries as ignored, and inserts a `Compaction` entry.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("compaction")]
pub struct CompactContext {
    /// The session to compact.
    pub session_id: SessionId,
}
