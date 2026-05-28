//! Compaction worker — summarizes conversation history into structured checkpoints.
//!
//! Implements [`HistoryWorker`] to produce [`HistoryMutation`] batches.
//! The compaction algorithm:
//! 1. Finds the start boundary (after the last compaction summary)
//! 2. Computes a cut index respecting the token reserve
//! 3. Adjusts the cut to avoid breaking tool loop boundaries
//! 4. Calls the LLM for summarization
//! 5. Produces mutations: `SetContextOverride(ForcedExclude)` for gathered entries,
//!    `InsertEntry` for the compaction summary

pub mod algorithm;
pub mod serializer;
pub mod worker;

pub use worker::{CompactionTrigger, CompactionWorker};
