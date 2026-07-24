//! jinn is an MCP *client*: it connects to externally-defined MCP servers
//! over stdio or HTTP. Servers are declared in `jinn.toml` under
//! `[[mcp_servers]]` and enabled per-session from the persisted `SessionCore`.
//!
//! See [`McpServerConfig`] and [`TransportKind`].

pub mod intent;
pub mod picker_entry;
pub mod render;

use serde::{Deserialize, Serialize};

/// How jinn connects to an MCP server.
///
/// - [`Stdio`](TransportKind::Stdio) — jinn spawns the server as a child
///   process and speaks JSON-RPC over its stdin/stdout. The default and the
///   universal fallback.
/// - [`Http`](TransportKind::Http) — jinn spawns the server as a child
///   process in HTTP mode, allocates a free local port, and connects to it
///   over `StreamableHTTP` at `http://<ip>:<port>/mcp`. Both stdout and
///   stderr are captured for the inspector logs pane (unlike stdio, stdout
///   is free for the server's own logs in this mode).
/// - [`RemoteHttp`](TransportKind::RemoteHttp) — jinn connects to an
///   already-running HTTP server at the given `url`; it owns no process.
///
/// The default is [`Stdio`](TransportKind::Stdio), which is what an absent
/// `transport` field in `jinn.toml` deserializes to, so existing stdio-only
/// configurations keep working unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TransportKind {
    /// JSON-RPC over the child process's stdin/stdout.
    #[serde(rename = "stdio")]
    #[default]
    Stdio,
    /// `StreamableHTTP` over a managed local child process.
    Http,
    /// `StreamableHTTP` to an externally-managed server at `url`.
    RemoteHttp {
        /// The full URL to connect to (e.g. `http://host:3001/mcp`).
        url: String,
    },
}

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
    /// Arguments passed to the command.
    ///
    /// For [`Http`](TransportKind::Http) servers, `<ip>` and `<port>`
    /// replacement tokens are expanded into the configured bind address and
    /// the allocated port respectively (e.g.
    /// `["server.js", "--port", "<port>", "--host", "<ip>"]`).
    #[serde(default)]
    pub args: Vec<String>,
    /// How jinn connects to this server. Defaults to
    /// [`Stdio`](TransportKind::Stdio) when absent, so existing stdio-only
    /// configurations keep working unchanged.
    #[serde(default)]
    pub transport: TransportKind,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_transport_defaults_to_stdio() {
        // Given a config TOML with no transport field.
        let toml = r#"
            name = "excalimate"
            command = "npx"
            args = ["@excalimate/mcp-server", "--stdio"]
        "#;

        // When deserializing.
        let config: McpServerConfig = toml::from_str(toml).expect("parse");

        // Then transport defaults to Stdio (backward-compat regression guard).
        assert_eq!(config.transport, TransportKind::Stdio);
    }

    #[test]
    fn http_transport_round_trips_through_toml() {
        // Given an Http-mode config.
        let config = McpServerConfig {
            name: "excalimate".to_owned(),
            command: "node".to_owned(),
            args: vec!["server.js", "--port", "<port>"].into_iter().map(String::from).collect(),
            transport: TransportKind::Http,
        };

        // When serializing and deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: McpServerConfig = toml::from_str(&toml).expect("deserialize");

        // Then the transport is preserved as Http.
        assert_eq!(back.transport, TransportKind::Http);
    }

    #[test]
    fn remote_http_transport_round_trips_through_toml() {
        // Given a RemoteHttp config with a URL.
        let config = McpServerConfig {
            name: "remote".to_owned(),
            command: String::new(),
            args: vec![],
            transport: TransportKind::RemoteHttp {
                url: "http://localhost:3001/mcp".to_owned(),
            },
        };

        // When serializing and deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: McpServerConfig = toml::from_str(&toml).expect("deserialize");

        // Then the transport + URL are preserved.
        assert_eq!(
            back.transport,
            TransportKind::RemoteHttp { url: "http://localhost:3001/mcp".to_owned() }
        );
    }

    #[test]
    fn stdio_transport_round_trips_through_toml() {
        // Given an explicit Stdio config.
        let config = McpServerConfig {
            name: "excalimate".to_owned(),
            command: "npx".to_owned(),
            args: vec!["--stdio"].into_iter().map(String::from).collect(),
            transport: TransportKind::Stdio,
        };

        // When serializing and deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: McpServerConfig = toml::from_str(&toml).expect("deserialize");

        // Then the transport is preserved as Stdio.
        assert_eq!(back.transport, TransportKind::Stdio);
    }
}
