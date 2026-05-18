//! Compaction event types.

use serde::{Deserialize, Serialize};

use crate::protocol::{EventMsg, SessionId};

/// Emitted when a compaction has completed successfully.
///
/// Other actors (e.g., session persistence) can listen for this
/// to know when to save the updated session state.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("compaction")]
pub struct CompactionCompleted {
    /// The session that was compacted.
    pub session_id: SessionId,
    /// How many entries were compacted (marked as ignored).
    pub entries_compacted: usize,
}
