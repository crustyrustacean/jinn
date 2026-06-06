//! Get time built-in tool - returns current UTC date/time.

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

use super::BoxedToolFuture;

/// Returns the tool definition for the `get_time` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "get_time".to_owned(),
        description: "Returns the current date and time in UTC.".to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {}
        }),
        server_tool_type: None,
    }
}

/// Executes the `get_time` built-in tool.
pub fn execute(call: ToolCall, _ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let now = jiff::Zoned::now();
        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: now.to_string(),
            success: true,
            full_content: None,
            truncation: None,
            pin_position: None,
        }
    })
}
