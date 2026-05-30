//! Compaction session data — state that persists across assembly invocations.
//!
//! In the full implementation, this will store compaction summaries keyed by
//! `ChatEntryId` ranges. For the stub, it carries a compaction counter.

use serde::{Deserialize, Serialize};

/// Placeholder session data for the compaction strategy.
///
/// Validates that the strategy state persistence plumbing works end-to-end
/// before the full implementation adds complex state (summaries, entry ranges).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSessionData {
    /// The number of compactions performed (for future use).
    compaction_count: usize,
}

impl CompactionSessionData {
    /// Create new empty session data.
    #[must_use]
    pub fn new() -> Self {
        Self {
            compaction_count: 0,
        }
    }
}

impl Default for CompactionSessionData {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn compaction_session_data_starts_at_zero() {
        // Given new compaction session data.
        let data = CompactionSessionData::new();

        // Then the compaction count is zero.
        assert_eq!(data.compaction_count, 0);
    }
}
