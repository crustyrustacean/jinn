//! Request body builder for OpenAI-compatible chat completions.
//!
//! Converts [`LlmMessage`] and [`ToolDefinition`] into the JSON body
//! expected by the OpenAI chat completions endpoint.

use serde::Serialize;

use crate::LlmMessage;
use crate::tool_types::ToolDefinition;

/// Top-level request body for OpenAI-compatible chat completions.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    pub stream: bool,
    /// Request usage data (token counts and cost) in streaming responses.
    pub stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoiceValue>,
    /// Extra body fields merged from config (e.g., `enable_thinking`).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Stream options for requesting usage data in streaming responses.
///
/// When `include_usage` is `true`, OpenAI-compatible providers include
/// `usage` (with token counts and cost) in the final SSE chunk.
#[derive(Debug, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

/// Tool choice parameter.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoiceValue {
    Auto,
    #[allow(dead_code)]
    None,
    #[allow(dead_code)]
    Required,
}

/// Builds a [`ChatCompletionRequest`] from protocol types.
///
/// Performs message sanitization required by strict OpenAI-compatible providers:
///
/// - All system messages are concatenated into a single leading system message.
/// - Consecutive messages with the same role (user, assistant) are merged by
///   joining their content with `\n\n`. This prevents "illegal messages" errors
///   from providers like ZAI that reject same-role adjacency.
/// - Assistant messages with tool calls use `null` content instead of empty
///   string when the text content is empty (required by ZAI).
pub fn build_request(
    model: &str,
    messages: &[LlmMessage],
    tools: &[ToolDefinition],
    extra_body: &serde_json::Map<String, serde_json::Value>,
) -> ChatCompletionRequest {
    // Concatenate all System messages into one system-role message.
    let mut system_contents: Vec<String> = Vec::new();
    let mut openai_messages: Vec<serde_json::Value> = Vec::new();
    for msg in messages {
        match msg {
            LlmMessage::System { content } => {
                system_contents.push(content.clone());
            }
            other => {
                openai_messages.push(message_to_json(other));
            }
        }
    }
    if !system_contents.is_empty() {
        openai_messages.insert(
            0,
            serde_json::json!({
                "role": "system",
                "content": system_contents.join("\n\n"),
            }),
        );
    }

    // Merge consecutive messages with the same role.
    // Many providers (ZAI, etc.) reject adjacent same-role messages.
    openai_messages = merge_consecutive_same_role(openai_messages);

    let openai_tools = if tools.is_empty() {
        None
    } else {
        Some(tools.iter().map(tool_definition_to_json).collect())
    };

    let tool_choice = if openai_tools.is_some() {
        Some(ToolChoiceValue::Auto)
    } else {
        None
    };

    ChatCompletionRequest {
        model: model.to_owned(),
        messages: openai_messages,
        stream: true,
        stream_options: StreamOptions {
            include_usage: true,
        },
        tools: openai_tools,
        tool_choice,
        extra: extra_body.clone(),
    }
}

/// Merge consecutive messages that share the same role.
///
/// Only `user` and `assistant` messages are merged. `tool` messages are
/// never merged (they are keyed by `tool_call_id`). Messages that have
/// `tool_calls` are also never merged (the tool_calls array is not
/// concatenatable).
fn merge_consecutive_same_role(messages: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    if messages.is_empty() {
        return messages;
    }

    let mut merged: Vec<serde_json::Value> = Vec::with_capacity(messages.len());
    merged.push(messages[0].clone());

    for msg in messages.iter().skip(1) {
        let prev = merged.last_mut().expect("merged is non-empty");
        let prev_role = prev.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let curr_role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

        // Only merge user+user or assistant+assistant, and only when
        // neither has tool_calls (those are structurally different).
        if (prev_role == "user" && curr_role == "user")
            || (prev_role == "assistant" && curr_role == "assistant"
                && !prev.get("tool_calls").is_some()
                && !msg.get("tool_calls").is_some())
        {
            // Concatenate content fields.
            let prev_content = prev
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            let curr_content = msg
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");

            // Use null for empty assistant content, string otherwise.
            let new_content = format!("{prev_content}\n\n{curr_content}");
            let new_content = if prev_role == "assistant" && new_content.trim().is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(new_content)
            };
            prev["content"] = new_content;
        } else {
            merged.push(msg.clone());
        }
    }

    merged
}

/// Convert an [`LlmMessage`] to an OpenAI-format JSON object.
fn message_to_json(msg: &LlmMessage) -> serde_json::Value {
    match msg {
        LlmMessage::System { content } => serde_json::json!({
            "role": "system",
            "content": content,
        }),
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
            let json_calls: Vec<serde_json::Value> = calls.iter().map(tool_call_to_json).collect();
            // Use null for empty assistant content with tool calls.
            // Some providers (ZAI) reject empty-string content.
            let content_value = if content.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(content.clone())
            };
            serde_json::json!({
                "role": "assistant",
                "content": content_value,
                "tool_calls": json_calls,
            })
        }
        LlmMessage::Tool {
            tool_call_id,
            content,
            ..
        } => serde_json::json!({
            "role": "tool",
            "tool_call_id": tool_call_id,
            "content": content,
        }),
    }
}

/// Convert a [`crate::tool_types::ToolCall`] to OpenAI-format JSON.
fn tool_call_to_json(tc: &crate::tool_types::ToolCall) -> serde_json::Value {
    serde_json::json!({
        "id": tc.id,
        "type": "function",
        "function": {
            "name": tc.name,
            "arguments": tc.arguments,
        },
    })
}

/// Convert a [`ToolDefinition`] to OpenAI-format JSON.
fn tool_definition_to_json(def: &ToolDefinition) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": def.name,
            "description": def.description,
            "parameters": def.parameters,
        },
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    #[rstest::rstest]
    fn build_request_includes_model_and_stream() {
        // Given messages and no tools.
        let messages = vec![LlmMessage::User {
            content: "hello".into(),
        }];

        // When building request.
        let req = build_request("gpt-4", &messages, &[], &serde_json::Map::new());

        // Then model and stream are correct.
        assert_eq!(req.model, "gpt-4");
        assert!(req.stream);
        assert!(req.tools.is_none());
    }

    #[rstest::rstest]
    fn build_request_includes_tools() {
        // Given messages and tool definitions.
        let messages = vec![LlmMessage::User {
            content: "what's the weather?".into(),
        }];
        let tools = vec![ToolDefinition {
            name: "get_weather".into(),
            description: "Get weather".into(),
            prompt_snippet: None,
            prompt_guidelines: vec![],
            parameters: serde_json::json!({"type": "object"}),
        }];

        // When building request.
        let req = build_request("gpt-4", &messages, &tools, &serde_json::Map::new());

        // Then tools are included and tool_choice is auto.
        assert!(req.tools.is_some());
        assert!(matches!(req.tool_choice, Some(ToolChoiceValue::Auto)));
    }

    #[rstest::rstest]
    fn build_request_merges_extra_body() {
        // Given extra_body with a custom field.
        let mut extra = serde_json::Map::new();
        extra.insert("enable_thinking".into(), serde_json::json!(true));

        // When building request.
        let req = build_request("gpt-4", &[], &[], &extra);

        // Then extra_body is merged.
        assert_eq!(req.extra.get("enable_thinking").unwrap(), true);
    }

    #[rstest::rstest]
    fn system_message_serializes_correctly() {
        let json = message_to_json(&LlmMessage::System {
            content: "You are helpful.".into(),
        });
        assert_eq!(json["role"], "system");
        assert_eq!(json["content"], "You are helpful.");
    }

    #[rstest::rstest]
    fn tool_result_message_serializes_correctly() {
        let json = message_to_json(&LlmMessage::Tool {
            tool_call_id: "call_1".into(),
            name: "echo".into(),
            content: "result".into(),
        });
        assert_eq!(json["role"], "tool");
        assert_eq!(json["tool_call_id"], "call_1");
        assert_eq!(json["content"], "result");
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
        let req = build_request("gpt-4", &messages, &[], &serde_json::Map::new());

        // Then messages has exactly 2 entries: system then user.
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0]["role"], "system");
        assert_eq!(
            req.messages[0]["content"].as_str().unwrap(),
            "First system.\n\nSecond system."
        );
        assert_eq!(req.messages[1]["role"], "user");
    }

    #[rstest::rstest]
    fn build_request_includes_stream_options() {
        // Given a basic request.
        let messages = vec![LlmMessage::User {
            content: "hello".into(),
        }];

        // When building request.
        let req = build_request("gpt-4", &messages, &[], &serde_json::Map::new());

        // Then stream_options requests usage data.
        assert!(req.stream_options.include_usage);
    }

    #[rstest::rstest]
    fn assistant_with_tool_calls_serializes_correctly() {
        let json = message_to_json(&LlmMessage::Assistant {
            content: String::new(),
            tool_calls: Some(vec![crate::tool_types::ToolCall {
                id: "call_1".into(),
                name: "echo".into(),
                arguments: r#"{"x":1}"#.into(),
            }]),
        });
        assert_eq!(json["role"], "assistant");
        // Empty content with tool calls should serialize as null.
        assert!(json["content"].is_null());
        let calls = json["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "echo");
    }

    #[rstest::rstest]
    fn assistant_with_tool_calls_and_nonempty_content_serializes_as_string() {
        let json = message_to_json(&LlmMessage::Assistant {
            content: "Let me check.".into(),
            tool_calls: Some(vec![crate::tool_types::ToolCall {
                id: "call_1".into(),
                name: "echo".into(),
                arguments: r#"{\"x\":1}"#.into(),
            }]),
        });
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"].as_str().unwrap(), "Let me check.");
    }

    #[rstest::rstest]
    fn merge_consecutive_user_messages() {
        let messages = vec![
            LlmMessage::User { content: "hello".into() },
            LlmMessage::User { content: "world".into() },
        ];
        let req = build_request("gpt-4", &messages, &[], &serde_json::Map::new());
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0]["role"], "user");
        let content = req.messages[0]["content"].as_str().unwrap();
        assert!(content.contains("hello"));
        assert!(content.contains("world"));
    }

    #[rstest::rstest]
    fn merge_consecutive_assistant_messages() {
        let messages = vec![
            LlmMessage::Assistant { content: "first".into(), tool_calls: None },
            LlmMessage::Assistant { content: "second".into(), tool_calls: None },
        ];
        let req = build_request("gpt-4", &messages, &[], &serde_json::Map::new());
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0]["role"], "assistant");
        let content = req.messages[0]["content"].as_str().unwrap();
        assert!(content.contains("first"));
        assert!(content.contains("second"));
    }

    #[rstest::rstest]
    fn merge_does_not_combine_tool_messages() {
        let messages = vec![
            LlmMessage::Tool {
                tool_call_id: "call_1".into(),
                name: "echo".into(),
                content: "result1".into(),
            },
            LlmMessage::Tool {
                tool_call_id: "call_2".into(),
                name: "echo".into(),
                content: "result2".into(),
            },
        ];
        let req = build_request("gpt-4", &messages, &[], &serde_json::Map::new());
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0]["tool_call_id"], "call_1");
        assert_eq!(req.messages[1]["tool_call_id"], "call_2");
    }

    #[rstest::rstest]
    fn merge_does_not_combine_assistant_with_tool_calls() {
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: Some(vec![crate::tool_types::ToolCall {
                    id: "call_1".into(),
                    name: "echo".into(),
                    arguments: "{}".into(),
                }]),
            },
            LlmMessage::Assistant {
                content: "plain text".into(),
                tool_calls: None,
            },
        ];
        let req = build_request("gpt-4", &messages, &[], &serde_json::Map::new());
        assert_eq!(req.messages.len(), 2);
    }
}
