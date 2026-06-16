//! Tool calling commands.

use serde::{Deserialize, Serialize};

use crate::feat::tools_actor::tool_types::{ToolCall, ToolDefinition};
use crate::protocol::SessionId;

use jiff::Timestamp;

/// Register tools that an actor can execute.
///
/// Sent by actors at startup to declare which tools they provide.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterTools {
    /// The name of the actor providing these tools.
    pub provider: String,
    /// The tool definitions being registered.
    pub definitions: Vec<ToolDefinition>,
}

/// Request execution of a batch of tool calls for a session.
///
/// Sent by the LLM actor when the LLM produces tool calls.
/// Routed to the tool orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolBatch {
    /// The session requesting tool execution.
    pub session_id: SessionId,
    /// The tool calls to execute.
    pub tool_calls: Vec<ToolCall>,
    /// When the original LLM request was dispatched.
    pub dispatched_at: Timestamp,
}

/// Execute a single tool call.
///
/// Sent by the tool orchestrator to the actor that registered the tool.
/// Carries the session ID so the provider actor can include it in its
/// response event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteTool {
    /// The session this execution belongs to.
    pub session_id: SessionId,
    /// The tool call to execute.
    pub tool_call: ToolCall,
    /// When the original LLM request was dispatched.
    pub dispatched_at: Timestamp,
}

/// Register tools provided by a Lua plugin.
///
/// Sent by the plugin dispatch actor when a plugin with tool definitions is loaded.
/// Carries the plugin name, target scope (global or session-attached), and tool definitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterPluginTools {
    /// The name of the plugin providing these tools.
    pub plugin_name: String,
    /// `None` for global plugins (registered everywhere), `Some(id)` for session-attached plugins.
    pub target: Option<crate::feat::plugin_system::SessionRegistryId>,
    /// The session these tools are attached to.
    /// `None` for global tools (broadcast to all sessions).
    /// `Some(session_id)` for attached tools (only that session should store them).
    pub session_id: Option<SessionId>,
    /// The tool definitions being registered.
    pub definitions: Vec<ToolDefinition>,
    /// If true, register the executor in the tools map WITHOUT publishing a
    /// visibility event. Used for attachable plugin tools whose handlers load
    /// globally at startup: execution is global, visibility is per-session.
    #[serde(default)]
    pub execution_only: bool,
}

impl crate::common::bus::BusMessage for RegisterPluginTools {}

/// Cancel all pending tool executions for a session.
///
/// Sent by the LLM actor when a stream is cancelled while tool results
/// are pending. Routed to the tool orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelToolBatch {
    /// The session whose tool executions should be cancelled.
    pub session_id: SessionId,
}

/// Execute a web-fetch tool call.
///
/// Sent by the tool orchestrator to the `WebFetchActor`.
/// Carries the session ID and the tool call with URL + options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteWebFetch {
    /// The session this execution belongs to.
    pub session_id: SessionId,
    /// The tool call containing URL and options.
    pub tool_call: ToolCall,
}

impl crate::common::bus::BusMessage for RegisterTools {}
impl crate::common::bus::BusMessage for ExecuteToolBatch {}
impl crate::common::bus::BusMessage for ExecuteTool {}
impl crate::common::bus::BusMessage for CancelToolBatch {}
impl crate::common::bus::BusMessage for ExecuteWebFetch {}
