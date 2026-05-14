//! Protocol-level LLM message types.
//!
//! [`LlmMessage`] is a serializable representation of conversation turns,
//! decoupled from any specific provider's message format.

use serde::{Deserialize, Serialize};

use crate::tool_types::ToolCall;

/// A single message in an LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum LlmMessage {
    /// A system-level instruction to the LLM.
    System {
        /// The system prompt content.
        content: String,
    },
    /// A message from the user.
    User {
        /// The text content of the message.
        content: String,
    },
    /// A message from the AI assistant.
    Assistant {
        /// The text content of the message.
        content: String,
        /// Tool calls the assistant wants to make, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
    },
    /// A tool result message.
    Tool {
        /// The ID of the tool call this result is for.
        tool_call_id: String,
        /// The name of the tool that was executed.
        name: String,
        /// The output content.
        content: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn backward_compat_user_deserialization() {
        // Given old-format JSON for a user message.
        let json = r#"{"role":"user","content":"hello"}"#;

        // When deserializing.
        let msg: LlmMessage = serde_json::from_str(json).expect("deserialize");

        // Then it produces the expected variant.
        assert_eq!(
            msg,
            LlmMessage::User {
                content: "hello".into()
            }
        );
    }

    #[rstest::rstest]
    fn backward_compat_assistant_deserialization() {
        // Given old-format JSON for an assistant message.
        let json = r#"{"role":"assistant","content":"hi"}"#;

        // When deserializing.
        let msg: LlmMessage = serde_json::from_str(json).expect("deserialize");

        // Then it produces the expected variant with no tool calls.
        assert_eq!(
            msg,
            LlmMessage::Assistant {
                content: "hi".into(),
                tool_calls: None,
            }
        );
    }
}
