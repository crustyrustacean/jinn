//! Task list subsystem — phased task tracking for agent sessions.
//!
//! Provides a structured task list with one level of nesting: phases contain tasks.
//! The data model is stored per-session on [`SessionCore`](crate::feat::session::chat_session::SessionCore)
//! and persists across restarts via the existing session serialization pipeline.

mod types;
pub mod tools;

pub mod add_phase;
pub mod add_task;
pub mod complete_task;
pub mod get_task_list;
pub mod get_phase;

#[cfg(test)]
mod types_tests;

pub use types::{Phase, PhaseId, Task, TaskId, TaskList, TaskListError, TaskPosition, TaskStatus};
