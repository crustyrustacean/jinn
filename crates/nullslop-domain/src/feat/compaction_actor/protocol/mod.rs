//! Compaction protocol types.

pub mod command;
pub mod event;

pub use command::{
    BeginCompaction, CompactContext, CompactionResult, EndCompaction, EnqueueCompaction,
};
