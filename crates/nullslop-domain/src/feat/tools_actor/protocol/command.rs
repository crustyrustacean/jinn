//! Tool calling commands.

use serde::{Deserialize, Serialize};

use crate::feat::tools_actor::tool_types::{ToolCall, ToolDefinition};
use crate::protocol::CommandMsg;
use crate::protocol::SessionId;

/// Register tools that an actor can execute.
///
/// Sent by actors at startup to declare which tools they provide.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("tool")]
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
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("tool")]
pub struct ExecuteToolBatch {
    /// The session requesting tool execution.
    pub session_id: SessionId,
    /// The tool calls to execute.
    pub tool_calls: Vec<ToolCall>,
}

/// Execute a single tool call.
///
/// Sent by the tool orchestrator to the actor that registered the tool.
/// Carries the session ID so the provider actor can include it in its
/// response event.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("tool")]
pub struct ExecuteTool {
    /// The session this execution belongs to.
    pub session_id: SessionId,
    /// The tool call to execute.
    pub tool_call: ToolCall,
}

/// Cancel all pending tool executions for a session.
///
/// Sent by the LLM actor when a stream is cancelled while tool results
/// are pending. Routed to the tool orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("tool")]
pub struct CancelToolBatch {
    /// The session whose tool executions should be cancelled.
    pub session_id: SessionId,
}

/// Execute a web-fetch tool call.
///
/// Sent by the tool orchestrator to the `WebFetchActor`.
/// Carries the session ID and the tool call with URL + options.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("tool")]
pub struct ExecuteWebFetch {
    /// The session this execution belongs to.
    pub session_id: SessionId,
    /// The tool call containing URL and options.
    pub tool_call: ToolCall,
}
