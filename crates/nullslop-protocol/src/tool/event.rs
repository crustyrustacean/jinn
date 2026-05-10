//! Tool calling events.

use serde::{Deserialize, Serialize};

use super::types::{ToolCall, ToolDefinition, ToolResult};
use crate::EventMsg;
use crate::SessionId;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn tool_batch_completed_roundtrip() {
        // Given a ToolBatchCompleted event.
        let event = ToolBatchCompleted {
            session_id: SessionId::new(),
            results: vec![ToolResult {
                tool_call_id: "call_1".into(),
                name: "echo".into(),
                content: "hi".into(),
                success: true,
            }],
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ToolBatchCompleted = serde_json::from_str(&json).expect("deserialize");

        // Then it matches.
        assert_eq!(back.results.len(), 1);
    }

    #[rstest::rstest]
    fn tool_execution_completed_roundtrip() {
        // Given a ToolExecutionCompleted event.
        let event = ToolExecutionCompleted {
            session_id: SessionId::new(),
            result: ToolResult {
                tool_call_id: "call_1".into(),
                name: "echo".into(),
                content: "ok".into(),
                success: true,
            },
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ToolExecutionCompleted = serde_json::from_str(&json).expect("deserialize");

        // Then it matches.
        assert_eq!(back.result.content, "ok");
    }

    #[rstest::rstest]
    fn tool_use_started_roundtrip() {
        // Given a ToolUseStarted event.
        let event = ToolUseStarted {
            session_id: SessionId::new(),
            index: 0,
            id: "call_1".into(),
            name: "echo".into(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ToolUseStarted = serde_json::from_str(&json).expect("deserialize");

        // Then it matches.
        assert_eq!(back.id, "call_1");
        assert_eq!(back.name, "echo");
    }

    #[rstest::rstest]
    fn tool_call_received_roundtrip() {
        // Given a ToolCallReceived event.
        let event = ToolCallReceived {
            session_id: SessionId::new(),
            tool_call: ToolCall {
                id: "call_1".into(),
                name: "echo".into(),
                arguments: "{}".into(),
            },
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ToolCallReceived = serde_json::from_str(&json).expect("deserialize");

        // Then it matches.
        assert_eq!(back.tool_call.id, "call_1");
    }

    #[rstest::rstest]
    fn tool_call_streaming_roundtrip() {
        // Given a ToolCallStreaming event.
        let event = ToolCallStreaming {
            session_id: SessionId::new(),
            index: 2,
            partial_json: r#"{"input":"he"#.into(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ToolCallStreaming = serde_json::from_str(&json).expect("deserialize");

        // Then it matches.
        assert_eq!(back.index, 2);
        assert_eq!(back.partial_json, r#"{"input":"he"#);
    }

    #[rstest::rstest]
    fn tools_registered_roundtrip() {
        // Given a ToolsRegistered event.
        let event = ToolsRegistered {
            provider: "echo".into(),
            definitions: vec![ToolDefinition {
                name: "echo".into(),
                description: "Echoes input".into(),
                parameters: serde_json::json!({"type": "object"}),
            }],
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ToolsRegistered = serde_json::from_str(&json).expect("deserialize");

        // Then it matches.
        assert_eq!(back.provider, "echo");
        assert_eq!(back.definitions.len(), 1);
    }

}
