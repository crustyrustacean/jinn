//! Translation between MCP server tools and jinn `ToolDefinition`s.
//!
//! Each MCP tool is exposed to the LLM under a namespaced name so that two
//! servers can never collide (excalimate's `create_scene` and another server's
//! `create_scene` stay distinct). The convention is the standard MCP client
//! namespacing:
//!
//! ```text
//! mcp__<server_name>__<tool_name>
//! ```
//!
//! The `<server_name>` segment is derived from the configured server's `name`
//! field, sanitized so it cannot introduce a `__` boundary (which would break
//! round-trip stripping).

use jinn_provider::ToolDefinition;

/// The full namespace prefix for a server's tools: `mcp__<server_name>__`.
#[must_use]
pub fn provider_prefix(server_name: &str) -> String {
    format!("mcp__{}__", server_name)
}

/// The `provider` string used when registering a server's tools with the
/// orchestrator (matches [`provider_prefix`]).
#[must_use]
pub fn provider_name(server_name: &str) -> String {
    provider_prefix(server_name)
}

/// The full namespaced tool name: `mcp__<server_name>__<tool_name>`.
#[must_use]
pub fn namespaced_tool_name(server_name: &str, tool_name: &str) -> String {
    format!("mcp__{server_name}__{tool_name}")
}

/// Strips the `mcp__<server_name>__` prefix from a namespaced tool name,
/// returning the original server-side tool name.
///
/// Returns `None` if `namespaced` does not carry this server's prefix, so
/// callers can use it as a membership check + strip in one pass.
#[must_use]
pub fn strip_namespace<'a>(server_name: &str, namespaced: &'a str) -> Option<&'a str> {
    namespaced.strip_prefix(&provider_prefix(server_name))
}

/// Maps an MCP server tool (`rmcp::model::Tool`) into a jinn `ToolDefinition`
/// under the `mcp__<server_name>__<tool>` namespace.
#[must_use]
pub fn map_tool(server_name: &str, mcp_tool: &rmcp::model::Tool) -> ToolDefinition {
    ToolDefinition {
        name: namespaced_tool_name(server_name, &mcp_tool.name),
        description: mcp_tool
            .description
            .as_deref()
            .unwrap_or("MCP server tool")
            .to_owned(),
        parameters: serde_json::Value::Object(mcp_tool.input_schema.as_ref().clone()),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        server_tool_type: None,
    }
}
