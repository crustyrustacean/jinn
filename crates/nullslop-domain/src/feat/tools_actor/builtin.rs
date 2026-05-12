//! Built-in tool registry.
//!
//! Wires each tool module into a list of (definition, execute) pairs
//! for registration by the tool orchestrator at activation.

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition};

use super::{
    BoxedToolFuture, builtin_bash, builtin_echo, builtin_get_time, builtin_read, builtin_write,
    edit,
};

/// A built-in tool entry: its definition paired with its execute function.
pub(super) type BuiltinToolEntry = (ToolDefinition, fn(ToolCall, ToolContext) -> BoxedToolFuture);

/// Returns the built-in tool definitions and their execute functions.
pub(super) fn builtin_tools() -> Vec<BuiltinToolEntry> {
    vec![
        (
            builtin_echo::definition(),
            builtin_echo::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
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
    ]
}
