//! Built-in tool registry.
//!
//! Wires each tool module into a list of (definition, execute) pairs
//! for registration by the tool orchestrator at activation.

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition};

use super::{
    BoxedToolFuture, builtin_bash, builtin_get_time, builtin_read, builtin_skill, builtin_write,
    edit,
};

use crate::feat::judge::{builtin_session_query, builtin_session_query_recent, builtin_task_complete, builtin_task_incomplete};

/// A built-in tool entry: its definition paired with its execute function.
pub type BuiltinToolEntry = (ToolDefinition, fn(ToolCall, ToolContext) -> BoxedToolFuture);

/// Returns the built-in tool definitions and their execute functions.
pub fn builtin_tools() -> Vec<BuiltinToolEntry> {
    vec![
        (
            builtin_get_time::definition(),
            builtin_get_time::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            builtin_bash::definition(),
            builtin_bash::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            builtin_read::definition(),
            builtin_read::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            builtin_write::definition(),
            builtin_write::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            edit::definition(),
            edit::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            builtin_skill::definition(),
            builtin_skill::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        // Judge tools — only injected into judge sessions via assemble_prompt,
        // but must be registered here so dispatch_tool_call can find them.
        (
            builtin_session_query::definition(),
            builtin_session_query::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            builtin_session_query_recent::definition(),
            builtin_session_query_recent::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            builtin_task_complete::definition(),
            builtin_task_complete::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            builtin_task_incomplete::definition(),
            builtin_task_incomplete::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
    ]
}
