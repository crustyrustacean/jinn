//! Tool calling types — definitions, calls, and results.

use serde::{Deserialize, Serialize};

/// A tool definition that describes a tool the LLM can invoke.
///
/// Actors register these at startup via `RegisterTools`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDefinition {
    /// The unique name of the tool (e.g., "`file_read`").
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    pub parameters: serde_json::Value,
}

/// A tool call requested by the LLM during a streaming response.
///
/// Contains the function name and JSON arguments the LLM wants to invoke.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    /// Unique identifier for this tool call (assigned by the LLM provider).
    pub id: String,
    /// The name of the function to call.
    pub name: String,
    /// The arguments as a JSON string.
    pub arguments: String,
}

/// The result of executing a tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolResult {
    /// The ID of the tool call this result is for.
    pub tool_call_id: String,
    /// The name of the tool that was executed.
    pub name: String,
    /// The output content.
    pub content: String,
    /// Whether execution succeeded.
    pub success: bool,
}
