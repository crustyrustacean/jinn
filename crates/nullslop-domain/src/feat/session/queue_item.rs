//! Session turn dispatch queue items.
//!
//! The queue drives all turn transitions through a single dispatch point.
//! Each item type maps to a specific action when processed by the queue
//! processor in the session actor.

use crate::protocol::ChatEntry;
use serde::{Deserialize, Serialize};

/// Items in the session turn dispatch queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueueItem {
    /// A user-submitted message to send to the LLM.
    UserMessage(ChatEntry),
    /// Continue after a tool batch — re-assemble prompt with updated history.
    ToolContinuation,
    /// Context compaction is needed before any further turns.
    CompactionNeeded {
        /// If true, ignore `reserve_tokens` and compact everything after start boundary.
        #[serde(default)]
        compact_all: bool,
    },
}
