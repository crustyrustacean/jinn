//! Request body builder for Anthropic Messages API.
//!
//! Converts [`LlmMessage`] and [`ToolDefinition`] into the JSON body
//! expected by Anthropic's `/v1/messages` endpoint.

use serde::Serialize;

use crate::LlmMessage;
use crate::tool_types::ToolDefinition;

/// Top-level request body for Anthropic Messages API.
#[derive(Debug, Serialize)]
pub struct MessagesRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
}

/// Builds a [`MessagesRequest`] from protocol types.
pub fn build_request(
    model: &str,
    messages: &[LlmMessage],
    tools: &[ToolDefinition],
    system_prompt: Option<&str>,
) -> MessagesRequest {
    // Separate system messages from conversation messages.
    // Anthropic uses a top-level `system` field.
    // Concatenate all System messages to avoid silently dropping any.
    let system_contents: Vec<String> = messages
        .iter()
        .filter_map(|m| match m {
            LlmMessage::System { content } => Some(content.clone()),
            _ => None,
        })
        .collect();
    let system_text = system_prompt
        .map(std::borrow::ToOwned::to_owned)
        .or_else(|| {
            if system_contents.is_empty() {
                None
            } else {
                Some(system_contents.join("\n\n"))
            }
        });

    // Non-system messages go into the messages array.
    let anthropic_messages: Vec<serde_json::Value> = messages
        .iter()
        .filter(|m| !matches!(m, LlmMessage::System { .. }))
        .map(message_to_json)
        .collect();

    let anthropic_tools = if tools.is_empty() {
        None
    } else {
        Some(tools.iter().map(tool_definition_to_json).collect())
    };

    let tool_choice = if anthropic_tools.is_some() {
        Some(serde_json::json!({"type": "auto"}))
    } else {
        None
    };

    MessagesRequest {
        model: model.to_owned(),
        messages: anthropic_messages,
        max_tokens: 8192,
        system: system_text,
        stream: true,
        tools: anthropic_tools,
        tool_choice,
    }
}

/// Convert an [`LlmMessage`] to an Anthropic-format message JSON.
fn message_to_json(msg: &LlmMessage) -> serde_json::Value {
    match msg {
        LlmMessage::System { .. } => {
            // System messages are handled separately — should never reach here.
            serde_json::json!({"role": "user", "content": []})
        }
        LlmMessage::User { content } => serde_json::json!({
            "role": "user",
            "content": content,
        }),
        LlmMessage::Assistant {
            content,
            tool_calls: None,
        } => serde_json::json!({
            "role": "assistant",
            "content": content,
        }),
        LlmMessage::Assistant {
            content,
            tool_calls: Some(calls),
        } => {
            let mut content_blocks: Vec<serde_json::Value> = Vec::new();

            // Include text content if non-empty.
            if !content.is_empty() {
                content_blocks.push(serde_json::json!({
                    "type": "text",
                    "text": content,
                }));
            }

            // Tool use blocks.
            for tc in calls {
                content_blocks.push(serde_json::json!({
                    "type": "tool_use",
                    "id": tc.id,
                    "name": tc.name,
                    "input": serde_json::from_str::<serde_json::Value>(&tc.arguments)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::default())),
                }));
            }

            serde_json::json!({
                "role": "assistant",
                "content": content_blocks,
            })
        }
        LlmMessage::Tool {
            tool_call_id,
            content,
            ..
        } => serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_call_id,
                "content": content,
            }],
        }),
    }
}

/// Convert a [`ToolDefinition`] to Anthropic-format tool JSON.
fn tool_definition_to_json(def: &ToolDefinition) -> serde_json::Value {
    serde_json::json!({
        "name": def.name,
        "description": def.description,
        "input_schema": def.parameters,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn build_request_extracts_system_prompt() {
        // Given messages with a system prompt.
        let messages = vec![
            LlmMessage::System {
                content: "You are helpful.".into(),
            },
            LlmMessage::User {
                content: "hello".into(),
            },
        ];

        // When building request.
        let req = build_request("claude-3", &messages, &[], None);

        // Then system is extracted and not in messages.
        assert_eq!(req.system.as_deref(), Some("You are helpful."));
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0]["role"], "user");
    }

    #[rstest::rstest]
    fn build_request_uses_explicit_system_over_message() {
        let messages = vec![LlmMessage::System {
            content: "from message".into(),
        }];
        let req = build_request("claude-3", &messages, &[], Some("from param"));
        assert_eq!(req.system.as_deref(), Some("from param"));
    }

    #[rstest::rstest]
    fn tool_result_message_uses_user_role() {
        let json = message_to_json(&LlmMessage::Tool {
            tool_call_id: "toolu_1".into(),
            name: "echo".into(),
            content: "result".into(),
        });
        assert_eq!(json["role"], "user");
        let content = json["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "toolu_1");
    }

    #[rstest::rstest]
    fn assistant_with_tool_calls_uses_content_blocks() {
        let json = message_to_json(&LlmMessage::Assistant {
            content: String::new(),
            tool_calls: Some(vec![crate::tool_types::ToolCall {
                id: "toolu_1".into(),
                name: "echo".into(),
                arguments: r#"{"x":1}"#.into(),
            }]),
        });
        assert_eq!(json["role"], "assistant");
        let content = json["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["name"], "echo");
    }

    #[rstest::rstest]
    fn build_request_concats_multiple_system_messages() {
        // Given messages with two System messages and a User message.
        let messages = vec![
            LlmMessage::System {
                content: "First system.".into(),
            },
            LlmMessage::System {
                content: "Second system.".into(),
            },
            LlmMessage::User {
                content: "hello".into(),
            },
        ];

        // When building request with no explicit system_prompt.
        let req = build_request("claude-3", &messages, &[], None);

        // Then system is the concatenation of both system messages.
        assert_eq!(
            req.system.as_deref(),
            Some("First system.\n\nSecond system.")
        );
        // And messages has exactly 1 entry (the User message).
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0]["role"], "user");
    }

    #[rstest::rstest]
    fn tool_definitions_use_input_schema() {
        let def = ToolDefinition {
            name: "echo".into(),
            description: "Echo".into(),
            prompt_snippet: None,
            prompt_guidelines: vec![],
            parameters: serde_json::json!({"type": "object"}),
            server_tool_type: None,
        };
        let json = tool_definition_to_json(&def);
        assert!(json.get("input_schema").is_some());
        assert!(json.get("parameters").is_none());
    }
}
