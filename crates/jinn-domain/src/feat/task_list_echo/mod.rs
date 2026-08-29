//! Task-list echo — synthetic prompt injection that keeps the task list
//! visible in LLM context.
//!
//! The model's task list lives in `SessionCore.task_list` and normally reaches
//! the model only through `todo_*` tool results, which are pruned from context
//! like any other history entry. This feature injects a synthetic
//! `[System]`-prefixed user message — the **echo** — positioned `echo_offset`
//! messages before the most recent message (snapped backward to the nearest
//! tool-loop boundary), rendered live from the session's task list at assembly
//! time.
//!
//! The echo is tree-only: [`render_next_block`]'s "→ NEXT" line is deliberately
//! excluded, because a next-step imperative injected alongside tangential work
//! makes the model act on the list instead of the user's request. Tool-result
//! formats are untouched — they keep their "→ NEXT" lines because a tool call
//! is deliberate task-list intent.
//!
//! Configuration lives in `jinn.toml` under `[task_list]`
//! ([`TaskListPreferences`]). An `echo_offset` of `0` disables injection.

pub mod render;
#[cfg(test)]
pub mod render_tests;

pub use render::echo_message;

/// Default number of messages between the echo and the tail of the assembled
/// prompt. Bounds the uncached tail window per send. Kept in sync with the
/// default in `TaskListPreferences`; the config accessors are authoritative.
pub const DEFAULT_ECHO_OFFSET: usize = 10;

/// Default maximum rendered tree lines in the echo before truncation.
pub const DEFAULT_ECHO_MAX_LINES: usize = 60;