//! Core types for prompt assembly.

use serde::{Deserialize, Serialize};

use crate::feat::context::strategy::compaction_data::CompactionSessionData;

/// Typed strategy state - carries compaction-specific persistent data.
///
/// Stored directly on `ChatSessionState` and serialized with the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StrategyState {
    Compaction(CompactionSessionData),
}
