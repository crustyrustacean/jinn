//! Per-session state machine for the LLM actor.
//!
//! Each active LLM conversation is tracked by a [`SessionData`] instance
//! that records the current state, accumulated messages, and stream data.

use nullslop_protocol::provider::LlmMessage;
use nullslop_protocol::tool::ToolCall;

/// Per-session state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionState {
    /// No active streaming.
    Idle,
    /// Streaming tokens from the LLM.
    Streaming,
    /// Tool calls were sent; awaiting results from the orchestrator.
    AwaitingToolResults,
}

/// Per-session data tracked by the actor.
pub(crate) struct SessionData {
    /// Current state in the streaming lifecycle.
    pub(crate) state: SessionState,
    /// Accumulated messages for the conversation (survives across tool loops).
    pub(crate) messages: Vec<LlmMessage>,
    /// Accumulated text content from the current stream.
    pub(crate) accumulated_text: String,
    /// Accumulated tool calls from the current stream.
    pub(crate) accumulated_tool_calls: Vec<ToolCall>,
}

impl SessionData {
    /// Creates a new [`SessionData`] with the given initial messages.
    pub(crate) fn new(messages: Vec<LlmMessage>) -> Self {
        Self {
            state: SessionState::Idle,
            messages,
            accumulated_text: String::new(),
            accumulated_tool_calls: Vec::new(),
        }
    }
}
