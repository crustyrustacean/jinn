//! Stream events from LLM chat with tool support.
//!
//! [`StreamEvent`] is our own type for streaming LLM responses, decoupled
//! from the `llm` crate's `StreamChunk`. All conversion happens inside
//! provider implementations.

use crate::protocol::tool::ToolCall;

/// A streaming event from an LLM chat response.
///
/// Produced by [`LlmService::chat_stream_with_tools`](super::LlmService::chat_stream_with_tools).
/// The stream always ends with a [`Done`](StreamEvent::Done) event.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// A text content delta.
    Text(String),
    /// A tool use block started (ID and name known, arguments streaming).
    ToolUseStart {
        /// The index of this content block in the response.
        index: usize,
        /// The unique ID for this tool call (assigned by the LLM provider).
        id: String,
        /// The name of the tool being called.
        name: String,
    },
    /// A partial JSON delta for tool call arguments.
    ToolUseInputDelta {
        /// The index of this content block.
        index: usize,
        /// Partial JSON string for the tool input.
        partial_json: String,
    },
    /// A tool use block completed with an assembled tool call.
    ToolUseComplete {
        /// The index of this content block.
        index: usize,
        /// The complete tool call.
        tool_call: ToolCall,
    },
    /// The stream ended.
    Done {
        /// Why the stream stopped (e.g., "`end_turn`", "`tool_use`").
        stop_reason: String,
    },
}
