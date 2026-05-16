//! Request body builder for Google Gemini API.
//!
//! Converts [`LlmMessage`] and [`ToolDefinition`] into the JSON body
//! expected by Google's `streamGenerateContent` endpoint.

use serde::Serialize;

use crate::LlmMessage;
use crate::tool_types::ToolDefinition;

/// Top-level request body for Google Gemini API.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiRequest {
    /// Conversation contents.
    pub contents: Vec<serde_json::Value>,
    /// System instruction (top-level field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<serde_json::Value>,
    /// Tool declarations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
}

/// Builds a [`GeminiRequest`] from protocol types.
pub fn build_request(messages: &[LlmMessage], tools: &[ToolDefinition]) -> GeminiRequest {
    // Extract system prompt separately.
    // Concatenate all System messages into one system instruction.
    // Google uses a top-level `systemInstruction` field.
    let system_contents: Vec<String> = messages
        .iter()
        .filter_map(|m| match m {
            LlmMessage::System { content } => Some(content.clone()),
            _ => None,
        })
        .collect();
    let system_instruction = if system_contents.is_empty() {
        None
    } else {
        Some(serde_json::json!({
            "parts": [{"text": system_contents.join("\n\n")}]
        }))
    };

    // Non-system messages → contents array.
    let contents: Vec<serde_json::Value> = messages
        .iter()
        .filter(|m| !matches!(m, LlmMessage::System { .. }))
        .map(message_to_json)
        .collect();

    let gemini_tools = if tools.is_empty() {
        None
    } else {
        Some(vec![serde_json::json!({
            "functionDeclarations": tools.iter().map(tool_definition_to_json).collect::<Vec<_>>()
        })])
    };

    GeminiRequest {
        contents,
        system_instruction,
        tools: gemini_tools,
    }
}

/// Convert an [`LlmMessage`] to a Gemini-format content JSON.
fn message_to_json(msg: &LlmMessage) -> serde_json::Value {
    match msg {
        LlmMessage::System { .. } => {
            // System messages handled separately — unreachable.
            serde_json::json!({"role": "user", "parts": []})
        }
        LlmMessage::User { content } => serde_json::json!({
            "role": "user",
            "parts": [{"text": content}]
        }),
        LlmMessage::Assistant {
            content,
            tool_calls: None,
        } => serde_json::json!({
            "role": "model",
            "parts": [{"text": content}]
        }),
        LlmMessage::Assistant {
            content,
            tool_calls: Some(calls),
        } => {
            let mut parts: Vec<serde_json::Value> = Vec::new();

            if !content.is_empty() {
                parts.push(serde_json::json!({"text": content}));
            }

            for tc in calls {
                parts.push(serde_json::json!({
                    "functionCall": {
                        "name": tc.name,
                        "args": serde_json::from_str::<serde_json::Value>(&tc.arguments)
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::default()))
                    }
                }));
            }

            serde_json::json!({
                "role": "model",
                "parts": parts,
            })
        }
        LlmMessage::Tool {
            tool_call_id: _,
            name,
            content,
        } => serde_json::json!({
            "role": "function",
            "parts": [{
                "functionResponse": {
                    "name": name,
                    "response": {
                        "name": name,
                        "content": serde_json::from_str::<serde_json::Value>(content)
                            .unwrap_or(serde_json::Value::String(content.clone()))
                    }
                }
            }],
        }),
    }
}

/// Convert a [`ToolDefinition`] to Gemini-format function declaration.
fn tool_definition_to_json(def: &ToolDefinition) -> serde_json::Value {
    let properties = def
        .parameters
        .get("properties")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let required = def
        .parameters
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(std::borrow::ToOwned::to_owned))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    serde_json::json!({
        "name": def.name,
        "description": def.description,
        "parameters": {
            "type": "object",
            "properties": properties,
            "required": required,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn build_request_extracts_system_instruction() {
        let messages = vec![
            LlmMessage::System {
                content: "Be helpful.".into(),
            },
            LlmMessage::User {
                content: "hello".into(),
            },
        ];

        let req = build_request(&messages, &[]);

        assert!(req.system_instruction.is_some());
        assert_eq!(req.contents.len(), 1);
        assert_eq!(req.contents[0]["role"], "user");
    }

    #[rstest::rstest]
    fn user_message_uses_user_role() {
        let json = message_to_json(&LlmMessage::User {
            content: "hi".into(),
        });
        assert_eq!(json["role"], "user");
    }

    #[rstest::rstest]
    fn assistant_message_uses_model_role() {
        let json = message_to_json(&LlmMessage::Assistant {
            content: "hey".into(),
            tool_calls: None,
        });
        assert_eq!(json["role"], "model");
    }

    #[rstest::rstest]
    fn tool_result_uses_function_role() {
        let json = message_to_json(&LlmMessage::Tool {
            tool_call_id: "call_1".into(),
            name: "echo".into(),
            content: "result".into(),
        });
        assert_eq!(json["role"], "function");
        let parts = json["parts"].as_array().unwrap();
        assert!(parts[0].get("functionResponse").is_some());
    }

    #[rstest::rstest]
    fn tool_definitions_use_function_declarations() {
        let def = ToolDefinition {
            name: "echo".into(),
            description: "Echo".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"input": {"type": "string"}},
                "required": ["input"]
            }),
        };
        let json = tool_definition_to_json(&def);
        assert_eq!(json["name"], "echo");
        assert!(json.get("parameters").is_some());
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

        // When building request.
        let req = build_request(&messages, &[]);

        // Then system_instruction is Some with concatenated text.
        assert!(req.system_instruction.is_some());
        let parts = req.system_instruction.as_ref().unwrap()["parts"].as_array().unwrap();
        assert_eq!(
            parts[0]["text"].as_str().unwrap(),
            "First system.\n\nSecond system."
        );
        // And contents has exactly 1 entry (the User message).
        assert_eq!(req.contents.len(), 1);
        assert_eq!(req.contents[0]["role"], "user");
    }

    #[rstest::rstest]
    fn assistant_with_tool_calls_includes_function_call_parts() {
        let json = message_to_json(&LlmMessage::Assistant {
            content: String::new(),
            tool_calls: Some(vec![crate::tool_types::ToolCall {
                id: "call_1".into(),
                name: "echo".into(),
                arguments: r#"{"x":1}"#.into(),
            }]),
        });
        assert_eq!(json["role"], "model");
        let parts = json["parts"].as_array().unwrap();
        assert!(parts[0].get("functionCall").is_some());
    }
}
