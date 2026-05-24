//! Compaction event types.

use serde::{Deserialize, Serialize};

use crate::protocol::{EventMsg, SessionId};

/// Emitted when a compaction has completed.
///
/// Other actors (e.g., the queue actor) can listen for this
/// to know when to dispatch a continuation after auto-compaction.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("compaction")]
pub struct CompactionCompleted {
    /// The session that was compacted.
    pub session_id: SessionId,
    /// How many entries were compacted (marked as ignored).
    pub entries_compacted: usize,
    /// Whether this was an automatically triggered compaction (not manual `/compact`).
    pub auto: bool,
}
