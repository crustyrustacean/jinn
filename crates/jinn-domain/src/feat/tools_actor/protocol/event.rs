//! Tool calling events.

use serde::{Deserialize, Serialize};

use crate::feat::tools_actor::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::protocol::EventMsg;
use crate::protocol::SessionId;

/// All tool calls in a batch have completed execution.
///
/// Emitted by the tool orchestrator when every tool call in a batch
/// has finished (success or failure). The LLM actor listens for this
/// to continue the multi-turn tool loop.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolBatchCompleted {
    /// The session this batch belongs to.
    pub session_id: SessionId,
    /// The results for each tool call in the batch.
    pub results: Vec<ToolResult>,
}

/// A single tool execution completed.
///
/// Emitted by provider actors after executing a tool.
/// The tool orchestrator aggregates these into a `ToolBatchCompleted`.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolExecutionCompleted {
    /// The session this execution belongs to.
    pub session_id: SessionId,
    /// The tool execution result.
    pub result: ToolResult,
}

/// Tools were registered by an actor.
///
/// Emitted after an actor sends `RegisterTools` to confirm registration.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolsRegistered {
    /// The name of the actor that registered tools.
    pub provider: String,
    /// The tool definitions that were registered.
    pub definitions: Vec<ToolDefinition>,
}

/// A tool call has started in the LLM stream (name and ID known, arguments pending).
///
/// Emitted by the LLM actor when the backend signals tool use start.
/// The chat log creates a placeholder entry for this tool call.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolUseStarted {
    /// The session this tool call belongs to.
    pub session_id: SessionId,
    /// The index of the tool call in the response.
    pub index: usize,
    /// The unique ID for this tool call (assigned by the LLM provider).
    pub id: String,
    /// The name of the tool being called.
    pub name: String,
}

/// A complete tool call received from the LLM stream.
///
/// Emitted by the LLM actor when a complete tool call arrives in the stream.
/// The chat log uses this to finalize the tool call entry.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolCallReceived {
    /// The session this tool call belongs to.
    pub session_id: SessionId,
    /// The assembled tool call.
    pub tool_call: ToolCall,
}

/// Streaming update for a tool call being assembled.
///
/// Emitted by the LLM actor as tool call arguments stream in.
/// The chat log uses this to render in-progress tool call arguments.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolCallStreaming {
    /// The session this tool call belongs to.
    pub session_id: SessionId,
    /// The index of the tool call in the response.
    pub index: usize,
    /// Partial JSON string for the tool arguments (accumulated so far).
    pub partial_json: String,
}

/// A tool has started executing.
///
/// Emitted by the tool orchestrator when a streaming tool begins actual execution
/// (after arguments are complete). The session actor creates a pending
/// ToolResult entry. Only emitted for streaming tools (e.g., bash).
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolExecutionStarted {
    /// The session this execution belongs to.
    pub session_id: SessionId,
    /// The unique ID for this tool call.
    pub tool_call_id: String,
    /// The name of the tool being executed.
    pub name: String,
}

/// Incremental output from a running tool.
///
/// Emitted by streaming tools as they produce output. Each event carries
/// a delta (new lines), not the accumulated total. The session actor
/// appends to the pending ToolResult entry's content.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolExecutionOutput {
    /// The session this output belongs to.
    pub session_id: SessionId,
    /// The tool call ID this output is for.
    pub tool_call_id: String,
    /// New output text (delta, not accumulated).
    pub output: String,
}

impl crate::common::bus::BusMessage for ToolBatchCompleted {}
impl crate::common::bus::BusMessage for ToolExecutionCompleted {}
impl crate::common::bus::BusMessage for ToolsRegistered {}
impl crate::common::bus::BusMessage for ToolUseStarted {}
impl crate::common::bus::BusMessage for ToolCallReceived {}
impl crate::common::bus::BusMessage for ToolCallStreaming {}
impl crate::common::bus::BusMessage for ToolExecutionStarted {}
impl crate::common::bus::BusMessage for ToolExecutionOutput {}
