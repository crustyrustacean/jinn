//! Built-in tool definitions and execute functions.
//!
//! Provides the four built-in tools (`echo`, `get_time`, `file_read`, `file_write`)
//! that are registered at actor activation. Each tool has a definition function
//! (returning a [`ToolDefinition`]) and an execute function (returning a future).

use crate::protocol::tool::{ToolCall, ToolDefinition, ToolResult};

use super::BoxedToolFuture;

/// A built-in tool entry: its definition paired with its execute function.
pub(super) type BuiltinToolEntry = (ToolDefinition, fn(ToolCall) -> BoxedToolFuture);

/// Returns the built-in tool definitions and their execute functions.
pub(super) fn builtin_tools() -> Vec<BuiltinToolEntry> {
    vec![
        (
            echo_definition(),
            execute_echo as fn(ToolCall) -> BoxedToolFuture,
        ),
        (
            get_time_definition(),
            execute_get_time as fn(ToolCall) -> BoxedToolFuture,
        ),
        (
            file_read_definition(),
            execute_file_read as fn(ToolCall) -> BoxedToolFuture,
        ),
        (
            file_write_definition(),
            execute_file_write as fn(ToolCall) -> BoxedToolFuture,
        ),
    ]
}

/// Returns the tool definition for the `echo` built-in tool.
pub(super) fn echo_definition() -> ToolDefinition {
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

/// Returns the tool definition for the `get_time` built-in tool.
pub(super) fn get_time_definition() -> ToolDefinition {
    ToolDefinition {
        name: "get_time".to_owned(),
        description: "Returns the current date and time in UTC.".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    }
}

/// Returns the tool definition for the `file_read` built-in tool.
pub(super) fn file_read_definition() -> ToolDefinition {
    ToolDefinition {
        name: "file_read".to_owned(),
        description: "Reads the contents of a file from disk.".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                }
            },
            "required": ["path"]
        }),
    }
}

/// Returns the tool definition for the `file_write` built-in tool.
pub(super) fn file_write_definition() -> ToolDefinition {
    ToolDefinition {
        name: "file_write".to_owned(),
        description: "Writes content to a file on disk, creating parent directories as needed."
            .to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        }),
    }
}

/// Executes the `echo` built-in tool.
pub(super) fn execute_echo(call: ToolCall) -> BoxedToolFuture {
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

/// Executes the `get_time` built-in tool.
pub(super) fn execute_get_time(call: ToolCall) -> BoxedToolFuture {
    Box::pin(async move {
        let now = jiff::Zoned::now();
        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: now.to_string(),
            success: true,
        }
    })
}

/// Executes the `file_read` built-in tool using async I/O.
pub(super) fn execute_file_read(call: ToolCall) -> BoxedToolFuture {
    Box::pin(async move {
        let path = match serde_json::from_str::<serde_json::Value>(&call.arguments) {
            Ok(args) => args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to parse arguments: {e}"),
                    success: false,
                };
            }
        };

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content,
                success: true,
            },
            Err(e) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to read file '{path}': {e}"),
                success: false,
            },
        }
    })
}

/// Executes the `file_write` built-in tool using async I/O.
///
/// Creates parent directories if they don't exist. Overwrites the file if it
/// already exists.
pub(super) fn execute_file_write(call: ToolCall) -> BoxedToolFuture {
    Box::pin(async move {
        let (path, content) = match serde_json::from_str::<serde_json::Value>(&call.arguments) {
            Ok(args) => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                (path, content)
            }
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to parse arguments: {e}"),
                    success: false,
                };
            }
        };

        if let Some(parent) = std::path::Path::new(&path).parent()
            && !parent.as_os_str().is_empty()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to create parent directories for '{path}': {e}"),
                success: false,
            };
        }

        match tokio::fs::write(&path, &content).await {
            Ok(()) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("wrote {} bytes to {path}", content.len()),
                success: true,
            },
            Err(e) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to write file '{path}': {e}"),
                success: false,
            },
        }
    })
}
