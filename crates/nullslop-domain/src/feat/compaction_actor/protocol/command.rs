//! Compaction command types.

use serde::{Deserialize, Serialize};

use crate::protocol::{CommandMsg, SessionId};

/// Request to compact the conversation context for a session.
///
/// The `CompactionActor` receives this command, gathers entries from the
/// last compaction boundary to the cut point, summarizes them via an LLM,
/// then emits [`BeginCompaction`] and [`EndCompaction`] commands for the
/// session actor to apply state mutations.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("compact-context")]
pub struct CompactContext {
    /// The session to compact.
    pub session_id: SessionId,
}

/// Marks entries as ignored and sets the session phase to Compacting.
///
/// Emitted by the compaction actor before calling the LLM.
/// Handled by the session actor, which is the sole state owner.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("begin-compaction")]
pub struct BeginCompaction {
    /// The session being compacted.
    pub session_id: SessionId,
    /// Indices of entries to mark as ignored.
    pub gathered_indices: Vec<usize>,
}

/// Inserts the compaction result and sets the session phase back to Idle.
///
/// Emitted by the compaction actor after the LLM call completes (success or failure).
/// Handled by the session actor, which is the sole state owner.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("end-compaction")]
pub struct EndCompaction {
    /// The session being compacted.
    pub session_id: SessionId,
    /// The compaction result — `None` means the LLM call failed.
    pub result: Option<CompactionResult>,
    /// Error message if the LLM call failed.
    pub error: Option<String>,
}

/// The result of a successful context compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    /// The compaction summary text from the LLM.
    pub summary: String,
    /// How many entries were compacted (marked as ignored).
    pub entries_compacted: usize,
    /// Estimated token count of the entries before compaction.
    pub tokens_before: usize,
    /// The model used for summarization.
    pub model_used: String,
    /// Insertion point for the compaction entry (right after the last gathered entry).
    pub boundary_index: usize,
}

/// Cancel an in-progress compaction for a session.
///
/// Aborts the in-flight LLM summarization task. The session actor's
/// `cancel_compacting()` handles resetting state; this command only
/// aborts the LLM call.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("cancel-compaction")]
pub struct CancelCompaction {
    /// The session whose compaction should be cancelled.
    pub session_id: SessionId,
}
