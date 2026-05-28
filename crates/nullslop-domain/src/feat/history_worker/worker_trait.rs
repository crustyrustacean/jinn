//! The `HistoryWorker` trait — pluggable heuristics for history mutation.
//!
//! Each worker inspects a snapshot of the session history and optionally
//! produces a batch of mutations. Workers run outside any lock.

use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::history_mutation::HistoryMutation;

/// A pluggable history mutation heuristic.
///
/// Each worker inspects a snapshot of the session history and optionally
/// produces a batch of mutations. Workers are run outside any lock,
/// so heuristic evaluation (including LLM calls) never blocks writes.
pub trait HistoryWorker: Send + Sync + 'static {
    /// Human-readable name for logging and diagnostics.
    fn name(&self) -> &str;

    /// Inspect the history snapshot and optionally produce mutations.
    ///
    /// Called outside any lock. The `history` parameter is an owned
    /// snapshot cloned under a brief read lock.
    fn evaluate(&self, history: &[ChatEntry]) -> Vec<HistoryMutation>;
}
