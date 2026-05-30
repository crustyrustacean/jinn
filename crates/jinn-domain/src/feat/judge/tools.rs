//! Judge-specific tools for evaluating origin session output.
//!
//! These tools are injected into judge sessions only. They allow the judge
//! to query the origin session's history and report verdicts.

pub mod session_query;
pub mod session_query_recent;
pub mod task_complete;
pub mod task_incomplete;

use crate::feat::tools_actor::BoxedToolFuture;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition};

/// A built-in tool entry: its definition paired with its execute function.
pub type BuiltinToolEntry = (ToolDefinition, fn(ToolCall, ToolContext) -> BoxedToolFuture);

/// Returns all judge tool definitions (for prompt injection).
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        session_query::definition(),
        session_query_recent::definition(),
        task_complete::definition(),
        task_incomplete::definition(),
    ]
}

/// Returns all judge tool entries (definition + execute function).
///
/// Used by the tool orchestrator to register judge tools at activation.
pub fn tool_entries() -> Vec<BuiltinToolEntry> {
    vec![
        (
            session_query::definition(),
            session_query::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            session_query_recent::definition(),
            session_query_recent::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            task_complete::definition(),
            task_complete::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            task_incomplete::definition(),
            task_incomplete::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
    ]
}
