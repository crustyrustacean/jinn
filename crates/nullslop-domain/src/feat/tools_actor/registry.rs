//! Built-in tool registry.
//!
//! Wires each tool module into a list of (definition, execute) pairs
//! for registration by the tool orchestrator at activation.

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition};

use super::{BoxedToolFuture, bash, edit, get_time, read, skill, write};

use crate::feat::judge::tools;
use crate::feat::todo_list;

/// A built-in tool entry: its definition paired with its execute function.
pub type BuiltinToolEntry = (ToolDefinition, fn(ToolCall, ToolContext) -> BoxedToolFuture);

/// Returns the built-in tool definitions and their execute functions.
pub fn builtin_tools() -> Vec<BuiltinToolEntry> {
    let mut entries = vec![
        (
            get_time::definition(),
            get_time::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            bash::definition(),
            bash::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            read::definition(),
            read::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            write::definition(),
            write::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            edit::definition(),
            edit::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            skill::definition(),
            skill::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
    ];
    entries.extend(todo_list::tools::tool_entries());
    entries.extend(tools::tool_entries());
    entries
}
