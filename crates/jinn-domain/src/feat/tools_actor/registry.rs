//! Built-in tool registry.
//!
//! Wires each tool module into a list of (definition, execute) pairs
//! for registration by the tool orchestrator at activation.

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition};

use super::{
    BoxedToolFuture, bash, edit, get_time, grep, interactive_term, interactive_term_kill,
    interactive_term_send, read, restart_mcp, save_plan, session_query, skill, write,
};
use crate::feat::todo_list;

/// A built-in tool entry: its definition paired with its execute function.
pub type BuiltinToolEntry = (ToolDefinition, fn(ToolCall, ToolContext) -> BoxedToolFuture);

/// Returns the built-in tool definitions and their execute functions.
///
/// `default_timeout_secs` flows into `bash::definition` so the resolved global timeout
/// is surfaced in the schema the model sees.
pub fn builtin_tools(default_timeout_secs: u64) -> Vec<BuiltinToolEntry> {
    let mut entries = vec![
        (
            get_time::definition(),
            get_time::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            bash::definition(default_timeout_secs),
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
            session_query::definition(),
            session_query::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            restart_mcp::definition(),
            restart_mcp::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            grep::definition(),
            grep::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            interactive_term::definition(),
            interactive_term::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            interactive_term_send::definition(),
            interactive_term_send::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            interactive_term_kill::definition(),
            interactive_term_kill::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
    ];
    entries.extend(todo_list::tools::tool_entries());

    entries
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]
    use super::builtin_tools;

    #[rstest::rstest]
    #[test]
    fn restart_mcp_server_is_registered() {
        // Given the builtin tool list.
        let tools = builtin_tools(30);
        let names: Vec<&str> = tools.iter().map(|(d, _)| d.name.as_str()).collect();

        // Then restart_mcp_server is present alongside the existing builtins.
        assert!(
            names.contains(&"restart_mcp_server"),
            "restart_mcp_server must be registered; got: {names:?}"
        );
        for required in [
            "get_time",
            "bash",
            "read",
            "write",
            "edit",
            "skill",
            "save_plan",
            "session_query",
            "grep",
            "interactive_term",
            "interactive_term_send",
            "interactive_term_kill",
        ] {
            assert!(
                names.contains(&required),
                "existing builtin {required} must still be present; got: {names:?}"
            );
        }
    }
}
