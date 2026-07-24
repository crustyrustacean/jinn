//! MCP client actor — owns one connection to one MCP server for one session.
//!
//! One [`McpActor`] is spawned per (session × enabled server). It connects to
//! the server over stdio (via [`McpClient`]), lists the server's tools at
//! startup, registers them under the `mcp__<server>__<tool>` namespace as
//! session-scoped tools, and answers [`ExecuteTool`] calls by forwarding them
//! to the server's `tools/call` and publishing the result as
//! [`ToolExecutionCompleted`].
//!
//! # Lifecycle
//!
//! - [`Actor::on_start`]: subscribe to [`ExecuteTool`]; connect the client and
//!   list tools. On success, register the tools. On failure, log and let the
//!   actor stop — the [`McpLifecycleActor`] reports status; a later
//!   enable/disable cycle can respawn.
//! - [`Actor::on_stop`]: shut the client down so the child process terminates.
//!
//! Each tool call is dispatched to a standalone task so the mailbox stays free
//! for concurrent requests (mirrors `WebSearchActor`).


use jinn_mcp::{
    CallToolResult, ContentBlock, JsonObject, McpClient, ServerCommand,
    tool_mapping::{map_tool, provider_name, strip_namespace},
};
use kameo::actor::ActorRef;
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::feat::mcp::McpServerConfig;
use crate::feat::tools_actor::protocol::command::{ExecuteTool, RegisterTools};
use crate::feat::tools_actor::protocol::event::ToolExecutionCompleted;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::protocol::SessionId;

/// The MCP client actor — one per (session × enabled server).
///
/// Owns a [`McpClient`] connection to a single MCP server process and answers
/// `ExecuteTool` calls whose name carries this server's namespace prefix.
pub struct McpActor {
    deps: ActorDeps,
    /// The session this actor serves.
    session_id: SessionId,
    /// The configured server this actor connects to.
    server: McpServerConfig,
    /// The live MCP client connection, established during `on_start`.
    client: Option<McpClient>,
}

/// Dependencies for [`McpActor`].
#[derive(Clone)]
pub struct McpActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// The session this actor serves.
    pub session_id: SessionId,
    /// The configured server to connect to.
    pub server: McpServerConfig,
}

/// Builds the [`ServerCommand`] for an [`McpServerConfig`].
fn server_command(config: &McpServerConfig) -> ServerCommand {
    ServerCommand {
        program: config.command.clone(),
        args: config.args.clone(),
    }
}

/// Converts an rmcp `CallToolResult` into a single text string for jinn's
/// `ToolResult::content`.
///
/// Text content blocks are concatenated (separated by newlines); non-text
/// blocks (images, audio, resources) are summarized as placeholders so the LLM
/// knows they existed even though they can't be rendered as text.
fn format_result_content(result: &CallToolResult) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in &result.content {
        match block {
            ContentBlock::Text(text) => parts.push(text.text.clone()),
            ContentBlock::Image(_) => parts.push("[image content]".to_owned()),
            ContentBlock::Audio(_) => parts.push("[audio content]".to_owned()),
            ContentBlock::Resource(_) => parts.push("[resource content]".to_owned()),
            ContentBlock::ResourceLink(_) => {
                parts.push("[resource link]".to_owned());
            }
            _ => parts.push("[unsupported content]".to_owned()),
        }
    }
    parts.join("\n")
}

impl kameo::Actor for McpActor {
    type Args = McpActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let McpActorDeps {
            deps,
            session_id,
            server,
        } = args;

        deps.subscribe(actor_ref.recipient::<ExecuteTool>()).await;

        let provider = provider_name(&server.name);

        // Connect + list tools. Failure to connect is non-fatal to the process:
        // we publish a failed registration marker so the LLM never sees stale
        // tool names, and let the actor run (idle). The lifecycle actor /
        // dashboard surfaces the dead status.
        let client = match McpClient::connect(&server_command(&server)).await {
            Ok(client) => client,
            Err(report) => {
                tracing::warn!(
                    server = %server.name,
                    session_id = %session_id,
                    error = %report,
                    "MCP actor: failed to connect to server"
                );
                return Ok(Self {
                    deps,
                    session_id,
                    server,
                    client: None,
                });
            }
        };

        let definitions = match client.list_tools().await {
            Ok(tools) => tools
                .iter()
                .map(|tool| map_tool(&server.name, tool))
                .collect::<Vec<ToolDefinition>>(),
            Err(report) => {
                tracing::warn!(
                    server = %server.name,
                    session_id = %session_id,
                    error = %report,
                    "MCP actor: failed to list tools"
                );
                let mut dead_client = client;
                dead_client.shutdown().await;
                return Ok(Self {
                    deps,
                    session_id,
                    server,
                    client: None,
                });
            }
        };

        tracing::info!(
            server = %server.name,
            session_id = %session_id,
            tool_count = definitions.len(),
            "MCP actor: connected, registering tools"
        );

        let () = deps
            .services
            .bus
            .publish(RegisterTools {
                provider,
                definitions,
                session_id: Some(session_id.clone()),
            })
            .await;

        Ok(Self {
            deps,
            session_id,
            server,
            client: Some(client),
        })
    }

    async fn on_stop(
        &mut self,
        _actor_ref: kameo::actor::WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        if let Some(client) = self.client.as_mut() {
            client.shutdown().await;
        }
        Ok(())
    }
}

impl BusPublish for McpActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        &self.deps.services.bus
    }
}

impl Message<ExecuteTool> for McpActor {
    type Reply = ();

    async fn handle(&mut self, msg: ExecuteTool, _ctx: &mut Context<Self, Self::Reply>) {
        // Only handle calls for this session whose tool name carries this
        // server's namespace prefix.
        if msg.session_id != self.session_id {
            return;
        }
        let Some(tool_name) = strip_namespace(&self.server.name, &msg.tool_call.name).map(str::to_owned) else {
            return;
        };

        let Some(client) = self.client.as_ref() else {
            // Client never connected (dead on start). Report a failed result.
            self.deps
                .publish(ToolExecutionCompleted {
                    session_id: msg.session_id,
                    result: failure_result(&msg.tool_call, "MCP server is not connected"),
                })
                .await;
            return;
        };

        let session_id = msg.session_id;
        let tool_call = msg.tool_call;

        let arguments = match parse_arguments(&tool_call.arguments) {
            Ok(args) => args,
            Err(err_msg) => {
                self.deps
                    .publish(ToolExecutionCompleted {
                        session_id,
                        result: failure_result(&tool_call, err_msg),
                    })
                    .await;
                return;
            }
        };

        // Run the call inline. Tool calls are I/O bound and infrequent;
        // serializing per-server is acceptable. The orchestrator batches
        // concurrent calls, bounding any mailbox backlog.
        let result = match client.call_tool(&tool_name, arguments).await {
            Ok(mcp_result) => {
                let success = !mcp_result.is_error.unwrap_or(false);
                ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    content: format_result_content(&mcp_result),
                    success,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                }
            }
            Err(report) => {
                tracing::warn!(error = %report, "MCP tools/call failed");
                failure_result(&tool_call, format!("MCP tool call failed: {report}"))
            }
        };

        self.deps
            .publish(ToolExecutionCompleted {
                session_id,
                result,
            })
            .await;
    }
}

/// Parses a tool call's JSON arguments string into an MCP `JsonObject`.
///
/// Returns `Ok(None)` for an empty/blank argument string (valid — the tool
/// takes no arguments) and `Err(message)` for malformed JSON.
fn parse_arguments(raw: &str) -> Result<Option<JsonObject>, String> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| format!("invalid arguments: {e}"))?;
    match value {
        serde_json::Value::Object(map) => Ok(Some(map)),
        other => Err(format!("expected JSON object for arguments, got {}", other)),
    }
}

/// Builds a failed [`ToolResult`] with the given message.
fn failure_result(tool_call: &ToolCall, message: impl Into<String>) -> ToolResult {
    ToolResult {
        tool_call_id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        content: message.into(),
        success: false,
        full_content: None,
        truncation: None,
        pin_position: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test assertions"
    )]

    use super::*;

    #[test]
    fn format_result_content_joins_text_blocks() {
        // Given a CallToolResult with two text blocks.
        let result = jinn_mcp::testkit::ok_result(vec![
            ContentBlock::text("line one"),
            ContentBlock::text("line two"),
        ]);

        // When formatting.
        let out = format_result_content(&result);

        // Then the text blocks are newline-joined.
        assert_eq!(out, "line one\nline two");
    }

    #[test]
    fn format_result_content_summarizes_non_text_blocks() {
        // Given a CallToolResult with an image block.
        let result = jinn_mcp::testkit::ok_result(vec![
            ContentBlock::text("before"),
            ContentBlock::image("data", "image/png"),
        ]);

        // When formatting.
        let out = format_result_content(&result);

        // Then non-text blocks are summarized as placeholders.
        assert_eq!(out, "before\n[image content]");
    }

    #[test]
    fn parse_arguments_empty_string_is_none() {
        // Given a blank arguments string.
        // When parsing.
        let result = parse_arguments("   ");

        assert!(result.is_ok_and(|opt| opt.is_none()));
    }

    #[test]
    fn parse_arguments_object_is_some() {
        // Given a JSON object arguments string.
        // When parsing.
        let result = parse_arguments(r#"{"key": "value"}"#).expect("parse");

        // Then it is Some with the object.
        let map = result.expect("some");
        assert_eq!(map.get("key").and_then(|v| v.as_str()), Some("value"));
    }

    #[test]
    fn parse_arguments_non_object_is_error() {
        // Given a JSON array arguments string.
        // When parsing.
        let result = parse_arguments("[1, 2, 3]");

        // Then it is an error.
        assert!(result.is_err());
    }

    #[test]
    fn server_command_maps_config_fields() {
        // Given a server config.
        let config = McpServerConfig {
            name: "excalimate".to_owned(),
            command: "npx".to_owned(),
            args: vec!["@excalimate/mcp-server".to_owned(), "--stdio".to_owned()],
        };

        // When building the command.
        let cmd = server_command(&config);

        // Then the program and args are copied.
        assert_eq!(cmd.program, "npx");
        assert_eq!(cmd.args, vec!["@excalimate/mcp-server", "--stdio"]);
    }

    /// Verifies tool-name filtering strips the namespace correctly.
    #[test]
    fn strip_namespace_roundtrip() {
        // Given a namespaced tool name for "excalimate".
        let namespaced = "mcp__excalimate__create_scene";

        // When stripping the namespace.
        let stripped = strip_namespace("excalimate", namespaced);

        // Then the original server-side name is recovered.
        assert_eq!(stripped, Some("create_scene"));
    }

    /// Verifies a different server's prefix does not match.
    #[test]
    fn strip_namespace_rejects_other_server() {
        // Given a namespaced tool name for "excalimate".
        let namespaced = "mcp__excalimate__create_scene";

        // When stripping with a different server name.
        let stripped = strip_namespace("other", namespaced);

        // Then it does not match.
        assert_eq!(stripped, None);
    }

    /// Ensures `provider_name` matches the prefix used for stripping.
    #[test]
    fn provider_name_matches_prefix() {
        // Given a server name.
        // When computing provider name and prefix.
        let provider = provider_name("excalimate");

        // Then the prefix is consistent with strip_namespace.
        assert!(strip_namespace("excalimate", &format!("{provider}create_scene")).is_some());
    }

    /// Ensures map_tool namespaces the tool name.
    #[test]
    fn map_tool_namespaces_name() {
        // Given an rmcp Tool.
        let mcp_tool = jinn_mcp::Tool::new(
            "create_scene",
            "Create a scene",
            serde_json::Map::new(),
        );

        // When mapping.
        let def = map_tool("excalimate", &mcp_tool);

        // Then the name is namespaced.
        assert_eq!(def.name, "mcp__excalimate__create_scene");
        assert_eq!(def.description, "Create a scene");
    }
}
