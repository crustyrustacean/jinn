//! Request body builder for OpenAI-compatible chat completions.
//!
//! Converts [`LlmMessage`] and [`ToolDefinition`] into the JSON body
//! expected by the OpenAI chat completions endpoint.

use serde::Serialize;

use crate::tool_types::ToolDefinition;
use crate::LlmMessage;

/// Top-level request body for OpenAI-compatible chat completions.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoiceValue>,
    /// Extra body fields merged from config (e.g., `enable_thinking`).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
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
pub fn build_request(
    model: &str,
    messages: &[LlmMessage],
    tools: &[ToolDefinition],
    extra_body: &serde_json::Map<String, serde_json::Value>,
) -> ChatCompletionRequest {
    let openai_messages: Vec<serde_json::Value> =
        messages.iter().map(message_to_json).collect();

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
        tools: openai_tools,
        tool_choice,
        extra: extra_body.clone(),
    }
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
            let json_calls: Vec<serde_json::Value> =
                calls.iter().map(tool_call_to_json).collect();
            serde_json::json!({
                "role": "assistant",
                "content": content,
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
        let calls = json["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "echo");
    }
}
