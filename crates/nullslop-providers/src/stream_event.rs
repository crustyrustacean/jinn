//! Stream events from LLM chat with tool support.
//!
//! [`StreamEvent`] is our own type for streaming LLM responses, decoupled
//! from the `llm` crate's `StreamChunk`. All conversion happens inside
//! provider implementations.

use nullslop_protocol::tool::ToolCall;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case::text(StreamEvent::Text("hello".to_owned()))]
    #[case::tool_start(StreamEvent::ToolUseStart {
        index: 0,
        id: "call_1".to_owned(),
        name: "echo".to_owned(),
    })]
    #[case::tool_delta(StreamEvent::ToolUseInputDelta {
        index: 0,
        partial_json: r#"{"input":"h"#.to_owned(),
    })]
    #[case::tool_complete(StreamEvent::ToolUseComplete {
        index: 0,
        tool_call: ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        },
    })]
    #[case::done(StreamEvent::Done {
        stop_reason: "end_turn".to_owned(),
    })]
    fn debug_formatting_produces_non_empty_string(#[case] event: StreamEvent) {
        // Given a StreamEvent variant.
        // When formatting with Debug.
        // Then the debug string is non-empty.
        assert!(!format!("{event:?}").is_empty());
    }

    #[rstest::rstest]
    #[case::text(
        StreamEvent::Text("hello".to_owned()),
        StreamEvent::Text("hello".to_owned())
    )]
    #[case::tool_start(
        StreamEvent::ToolUseStart {
            index: 0,
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
        },
        StreamEvent::ToolUseStart {
            index: 0,
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
        }
    )]
    #[case::tool_delta(
        StreamEvent::ToolUseInputDelta {
            index: 0,
            partial_json: r#"{"x"#.to_owned(),
        },
        StreamEvent::ToolUseInputDelta {
            index: 0,
            partial_json: r#"{"x"#.to_owned(),
        }
    )]
    #[case::tool_complete(
        StreamEvent::ToolUseComplete {
            index: 0,
            tool_call: ToolCall {
                id: "call_1".to_owned(),
                name: "echo".to_owned(),
                arguments: "{}".to_owned(),
            },
        },
        StreamEvent::ToolUseComplete {
            index: 0,
            tool_call: ToolCall {
                id: "call_1".to_owned(),
                name: "echo".to_owned(),
                arguments: "{}".to_owned(),
            },
        }
    )]
    #[case::done(
        StreamEvent::Done {
            stop_reason: "tool_use".to_owned(),
        },
        StreamEvent::Done {
            stop_reason: "tool_use".to_owned(),
        }
    )]
    fn partial_eq_compares_identical_variants_as_equal(
        #[case] a: StreamEvent,
        #[case] b: StreamEvent,
    ) {
        // Given two independently constructed identical StreamEvents.
        // When comparing with ==.
        // Then they are equal.
        assert_eq!(a, b);
    }
}
