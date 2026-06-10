//! Built-in tool registry.
//!
//! Wires each tool module into a list of (definition, execute) pairs
//! for registration by the tool orchestrator at activation.

use crate::feat::preferences_actor::user_preferences::BashConfig;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition};

use super::{BoxedToolFuture, bash, edit, get_time, grep, read, save_plan, skill, write};

use crate::feat::todo_list;

/// A built-in tool entry: its definition paired with its execute function.
pub type BuiltinToolEntry = (ToolDefinition, fn(ToolCall, ToolContext) -> BoxedToolFuture);

/// Returns the built-in tool definitions and their execute functions.
///
/// `bash_config` flows into `bash::definition` so the resolved default timeout
/// is surfaced in the schema the model sees.
pub fn builtin_tools(bash_config: &BashConfig) -> Vec<BuiltinToolEntry> {
    let mut entries = vec![
        (
            get_time::definition(),
            get_time::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            bash::definition(bash_config),
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
        (
            save_plan::definition(),
            save_plan::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            grep::definition(),
            grep::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
    ];
    entries.extend(todo_list::tools::tool_entries());

    entries
}
