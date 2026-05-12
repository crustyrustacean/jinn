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

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn register_tools_roundtrip() {
        // Given a RegisterTools command.
        let cmd = RegisterTools {
            provider: "echo".into(),
            definitions: vec![ToolDefinition {
                name: "echo".into(),
                description: "Echoes input".into(),
                parameters: serde_json::json!({"type": "object"}),
            }],
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: RegisterTools = serde_json::from_str(&json).expect("deserialize");

        // Then it matches.
        assert_eq!(back.provider, "echo");
        assert_eq!(back.definitions.len(), 1);
    }

    #[rstest::rstest]
    fn execute_tool_batch_roundtrip() {
        // Given an ExecuteToolBatch command.
        let cmd = ExecuteToolBatch {
            session_id: SessionId::new(),
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "echo".into(),
                arguments: "{}".into(),
            }],
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: ExecuteToolBatch = serde_json::from_str(&json).expect("deserialize");

        // Then it matches.
        assert_eq!(back.tool_calls.len(), 1);
    }

    #[rstest::rstest]
    fn execute_tool_roundtrip() {
        // Given an ExecuteTool command.
        let cmd = ExecuteTool {
            session_id: SessionId::new(),
            tool_call: ToolCall {
                id: "call_1".into(),
                name: "echo".into(),
                arguments: r#"{"input":"hi"}"#.into(),
            },
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: ExecuteTool = serde_json::from_str(&json).expect("deserialize");

        // Then it matches.
        assert_eq!(back.tool_call.name, "echo");
    }

    #[rstest::rstest]
    fn cancel_tool_batch_roundtrip() {
        // Given a CancelToolBatch command.
        let cmd = CancelToolBatch {
            session_id: SessionId::new(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: CancelToolBatch = serde_json::from_str(&json).expect("deserialize");

        // Then the session_id field is preserved.
        assert_eq!(
            cmd.session_id, back.session_id,
            "session_id should roundtrip"
        );
    }
}
