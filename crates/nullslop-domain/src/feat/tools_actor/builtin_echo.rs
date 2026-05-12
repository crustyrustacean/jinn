//! Echo built-in tool — echoes input text back.

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

use super::BoxedToolFuture;

/// Returns the tool definition for the `echo` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "echo".to_owned(),
        description: "Echoes the input text back as the result.".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Text to echo back"
                }
            },
            "required": ["input"]
        }),
    }
}

/// Executes the `echo` built-in tool.
pub fn execute(call: ToolCall, _ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        match serde_json::from_str::<serde_json::Value>(&call.arguments) {
            Ok(args) => {
                let input = args.get("input").and_then(|v| v.as_str()).unwrap_or("");
                ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: input.to_owned(),
                    success: true,
                }
            }
            Err(e) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to parse arguments: {e}"),
                success: false,
            },
        }
    })
}
