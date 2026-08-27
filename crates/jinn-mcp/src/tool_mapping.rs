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
    format!("mcp__{server_name}__")
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]

    use super::*;
    use rmcp::model::Tool;

    fn mcp_tool(name: &str, desc: &str) -> Tool {
        Tool::new(name.to_owned(), desc.to_owned(), serde_json::Map::new())
    }

    #[rstest::rstest]
    #[test]
    fn provider_prefix_is_namespaced() {
        // Given a server name.
        // When computing the prefix.
        let prefix = provider_prefix("excalimate");

        // Then it is the standard mcp__<server>__ shape.
        assert_eq!(prefix, "mcp__excalimate__");
    }

    #[rstest::rstest]
    #[test]
    fn namespaced_tool_name_joins_server_and_tool() {
        // Given server + tool names.
        // When namespacing.
        let name = namespaced_tool_name("excalimate", "create_scene");

        // Then both segments appear in order.
        assert_eq!(name, "mcp__excalimate__create_scene");
    }

    #[rstest::rstest]
    #[test]
    fn strip_namespace_recovers_tool_name() {
        // Given a namespaced name for "excalimate".
        // When stripping.
        let stripped = strip_namespace("excalimate", "mcp__excalimate__create_scene");

        // Then the server-side tool name is recovered.
        assert_eq!(stripped, Some("create_scene"));
    }

    #[rstest::rstest]
    #[test]
    fn strip_namespace_rejects_other_server() {
        // Given a namespaced name for "excalimate".
        // When stripping with a different server.
        let stripped = strip_namespace("other", "mcp__excalimate__create_scene");

        // Then it does not match.
        assert_eq!(stripped, None);
    }

    #[rstest::rstest]
    #[test]
    fn two_servers_get_distinct_namespaces() {
        // Given the same tool name on two different servers.
        let alpha = namespaced_tool_name("alpha", "create_scene");
        let beta = namespaced_tool_name("beta", "create_scene");

        // Then the namespaced names differ.
        assert_ne!(alpha, beta);
        // And each strips back to the bare tool name under its own server.
        assert_eq!(strip_namespace("alpha", &alpha), Some("create_scene"));
        assert_eq!(strip_namespace("beta", &beta), Some("create_scene"));
    }

    #[rstest::rstest]
    #[test]
    fn map_tool_namespaces_name_and_preserves_description() {
        // Given an rmcp tool.
        let tool = mcp_tool("create_scene", "Create a scene");

        // When mapping.
        let def = map_tool("excalimate", &tool);

        // Then the name is namespaced and the description is preserved.
        assert_eq!(def.name, "mcp__excalimate__create_scene");
        assert_eq!(def.description, "Create a scene");
    }
}
