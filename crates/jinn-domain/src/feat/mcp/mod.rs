//! jinn is an MCP *client*: it connects to externally-defined MCP servers
//! over stdio or HTTP. Servers are declared in `jinn.toml` under
//! `[[mcp_server]]` and enabled per-session from the persisted `SessionCore`.
//!
//! See [`McpServerConfig`] and [`TransportKind`].

pub mod intent;
pub mod picker_entry;
pub mod render;

use serde::{Deserialize, Serialize};

/// How jinn connects to an MCP server.
///
/// Serialized as a flat string field (`transport = "stdio"`, `transport = "local_http"`,
/// etc.), not a sub-table. The default is [`Stdio`](TransportKind::Stdio), which is what an
/// absent `transport` field in `jinn.toml` deserializes to, so existing stdio-only
/// configurations keep working unchanged.
///
/// - [`Stdio`](TransportKind::Stdio) — jinn spawns the server as a child
///   process and speaks JSON-RPC over its stdin/stdout. The default and the
///   universal fallback.
/// - [`LocalHttp`](TransportKind::LocalHttp) — jinn spawns the server as a child
///   process in HTTP mode, allocates a free local port, and connects to it
///   over `StreamableHTTP` at the expanded `url`. Both stdout and stderr are
///   captured for the inspector logs pane (unlike stdio, stdout is free for
///   the server's own logs in this mode). The `url` field carries a template
///   with a `<port>` token; the host portion is the bind address and `<port>`
///   is expanded with the jinn-allocated port.
/// - [`RemoteHttp`](TransportKind::RemoteHttp) — jinn connects to an
///   already-running HTTP server at the `url`; it owns no process.
///
/// See [`McpServerConfig`] for the `url` and `command` fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    /// JSON-RPC over the child process's stdin/stdout.
    #[default]
    Stdio,
    /// `StreamableHTTP` over a managed local child process.
    LocalHttp,
    /// `StreamableHTTP` to an externally-managed server at `url`.
    RemoteHttp,
}

/// One configured MCP server.
///
/// Defined in `jinn.toml` under `[[mcp_server]]`. The `name` field is the
/// array key the `DocumentPatcher` matches entries by, and it doubles as the
/// per-session enablement identifier stored in `SessionCore::enabled_mcp_servers`
/// and the tool-namespace segment (`mcp__<name>__<tool>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this server. Used in the tool namespace
    pub name: String,
    /// Executable command to launch the server (e.g. `"npx"`).
    ///
    /// Optional: `None` is valid for [`RemoteHttp`](TransportKind::RemoteHttp)
    /// servers, which jinn never spawns.
    #[serde(default)]
    pub command: Option<String>,
    /// Arguments passed to the command.
    ///
    /// For [`LocalHttp`](TransportKind::LocalHttp) servers, `<ip>` and `<port>`
    /// replacement tokens are expanded into the configured bind address (parsed
    /// from `url`) and the allocated port respectively (e.g.
    /// `["server.js", "--port", "<port>", "--host", "<ip>"]`).
    #[serde(default)]
    pub args: Vec<String>,
    /// How jinn connects to this server. Defaults to
    /// [`Stdio`](TransportKind::Stdio) when absent, so existing stdio-only
    /// configurations keep working unchanged.
    #[serde(default)]
    pub transport: TransportKind,
    /// The HTTP URL template (`LocalHttp`) or full URL (`RemoteHttp`).
    ///
    /// For [`LocalHttp`](TransportKind::LocalHttp), the host portion of this
    /// template is the bind address, and `<port>` is expanded with the
    /// jinn-allocated port (e.g. `"http://127.0.0.1:<port>/mcp"`).
    ///
    /// For [`RemoteHttp`](TransportKind::RemoteHttp), this is the full URL of
    /// an externally-managed server (e.g. `"http://localhost:3001/mcp"`).
    ///
    /// Unused for [`Stdio`](TransportKind::Stdio).
    #[serde(default)]
    pub url: Option<String>,
}

impl McpServerConfig {
    /// One-line description for the MCP server picker.
    ///
    /// Shows the launch command and args so the user can tell servers apart
    /// and spot misconfiguration at a glance.
    #[must_use]
    pub fn description_for_picker(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(cmd) = &self.command {
            parts.push(cmd.clone());
        }
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]
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
    fn local_http_transport_round_trips_through_toml() {
        // Given a LocalHttp-mode config.
        let config = McpServerConfig {
            name: "excalimate".to_owned(),
            command: Some("node".to_owned()),
            args: vec!["server.js", "--port", "<port>"]
                .into_iter()
                .map(String::from)
                .collect(),
            transport: TransportKind::LocalHttp,
            url: Some("http://127.0.0.1:<port>/mcp".to_owned()),
        };

        // When serializing and deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: McpServerConfig = toml::from_str(&toml).expect("deserialize");

        // Then the transport is preserved as LocalHttp.
        assert_eq!(back.transport, TransportKind::LocalHttp);
    }

    #[test]
    fn remote_http_transport_round_trips_through_toml() {
        // Given a RemoteHttp config with a URL.
        let config = McpServerConfig {
            name: "remote".to_owned(),
            command: None,
            args: vec![],
            transport: TransportKind::RemoteHttp,
            url: Some("http://localhost:3001/mcp".to_owned()),
        };

        // When serializing and deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: McpServerConfig = toml::from_str(&toml).expect("deserialize");

        // Then the transport + URL are preserved.
        assert_eq!(back.transport, TransportKind::RemoteHttp);
        assert_eq!(
            back.url.as_deref(),
            Some("http://localhost:3001/mcp")
        );
    }

    #[test]
    fn stdio_transport_round_trips_through_toml() {
        // Given an explicit Stdio config.
        let config = McpServerConfig {
            name: "excalimate".to_owned(),
            command: Some("npx".to_owned()),
            args: vec!["--stdio"].into_iter().map(String::from).collect(),
            transport: TransportKind::Stdio,
            url: None,
        };

        // When serializing and deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: McpServerConfig = toml::from_str(&toml).expect("deserialize");

        // Then the transport is preserved as Stdio.
        assert_eq!(back.transport, TransportKind::Stdio);
    }

    #[test]
    fn command_is_optional_deserializes_when_absent() {
        // Given a config TOML with no command field.
        let toml = r#"
            name = "remote"
            transport = "remote_http"
            url = "http://localhost:3001/mcp"
        "#;

        // When deserializing.
        let config: McpServerConfig = toml::from_str(toml).expect("parse");

        // Then command is None.
        assert!(config.command.is_none());
    }

    #[test]
    fn remote_http_config_needs_no_command() {
        // Given a RemoteHttp config with command set to None.
        let config = McpServerConfig {
            name: "remote".to_owned(),
            command: None,
            args: vec![],
            transport: TransportKind::RemoteHttp,
            url: Some("http://localhost:3001/mcp".to_owned()),
        };

        // When serializing and deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: McpServerConfig = toml::from_str(&toml).expect("deserialize");

        // Then command stays None and the config round-trips.
        assert!(back.command.is_none());
        assert_eq!(back.transport, TransportKind::RemoteHttp);
    }
}
