//! MCP (Model Context Protocol) server configuration.
//!
//! jinn is an MCP *client*: it connects to externally-defined MCP servers over
//! stdio (and, in a follow-up, HTTP). Servers are declared in `jinn.toml` under
//! `[[mcp_servers]]` and enabled per-session from the persisted `SessionCore`.
//!
//! See [`McpServerConfig`].

pub mod intent;
pub mod picker_entry;
pub mod render;

use serde::{Deserialize, Serialize};

/// One configured MCP server.
///
/// Defined in `jinn.toml` under `[[mcp_servers]]`. The `name` field is the
/// array key the `DocumentPatcher` matches entries by, and it doubles as the
/// per-session enablement identifier stored in `SessionCore::enabled_mcp_servers`
/// and the tool-namespace segment (`mcp__<name>__<tool>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this server. Used in the tool namespace
    /// (`mcp__<name>__<tool>`) and in per-session enablement sets.
    pub name: String,
    /// Executable command to launch the server (e.g. `"npx"`).
    pub command: String,
    /// Arguments passed to the command (e.g. `["@excalimate/mcp-server", "--stdio"]`).
    #[serde(default)]
    pub args: Vec<String>,
}

impl McpServerConfig {
    /// One-line description for the MCP server picker.
    ///
    /// Shows the launch command and args so the user can tell servers apart
    /// and spot misconfiguration at a glance.
    #[must_use]
    pub fn description_for_picker(&self) -> String {
        let mut parts = vec![self.command.clone()];
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }
}
