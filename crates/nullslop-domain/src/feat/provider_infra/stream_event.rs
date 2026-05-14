//! Stream events from LLM chat with tool support.
//!
//! [`StreamEvent`] is our own type for streaming LLM responses, decoupled
//! from the `llm` crate's `StreamChunk`. All conversion happens inside
//! provider implementations.

use crate::feat::tools_actor::tool_types::ToolCall;

/// Why the stream stopped.
///
/// Parsed from the vendor `llm` crate's raw string at the conversion boundary.
/// Unknown values are preserved via [`StopReason::Other`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// The model requested tool use.
    ToolUse,
    /// The model finished generating normally.
    EndTurn,
    /// An unrecognized stop reason from the provider.
    Other(String),
}

impl From<&str> for StopReason {
    fn from(s: &str) -> Self {
        match s {
            "tool_use" => Self::ToolUse,
            "end_turn" => Self::EndTurn,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolUse => write!(f, "tool_use"),
            Self::EndTurn => write!(f, "end_turn"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

/// A streaming event from an LLM chat response.
///
/// Produced by [`LlmService::chat_stream_with_tools`](super::LlmService::chat_stream_with_tools).
/// The stream always ends with a [`Done`](StreamEvent::Done) event.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// A text content delta.
    Text(String),
    /// A reasoning/thinking content delta.
    Reasoning(String),
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
        /// Why the stream stopped.
        stop_reason: StopReason,
    },
}
