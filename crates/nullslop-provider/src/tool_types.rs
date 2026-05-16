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
    /// A one-line summary shown in the "Available tools" section of the system prompt.
    #[serde(default)]
    pub prompt_snippet: Option<String>,
    /// Behavioral guidelines injected into the "Tool guidelines" section of the system prompt.
    #[serde(default)]
    pub prompt_guidelines: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn tool_definition_roundtrips_through_serde() {
        let def = ToolDefinition {
            name: "file_read".to_owned(),
            description: "Read a file".to_owned(),
            parameters: serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            prompt_snippet: Some("Read file contents".to_owned()),
            prompt_guidelines: vec!["Use read to examine files.".to_owned()],
        };
        let json = serde_json::to_string(&def).expect("serialize");
        let back: ToolDefinition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, def);
    }

    #[rstest::rstest]
    fn tool_definition_deserializes_without_new_fields() {
        // Given a JSON ToolDefinition without prompt_snippet or prompt_guidelines.
        let json = r#"{"name":"file_read","description":"Read a file","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}"#;

        // When deserializing.
        let def: ToolDefinition = serde_json::from_str(json).expect("deserialize");

        // Then the new fields default to None / empty.
        assert_eq!(def.name, "file_read");
        assert_eq!(def.prompt_snippet, None);
        assert!(def.prompt_guidelines.is_empty());
    }

    #[rstest::rstest]
    fn tool_call_roundtrips_through_serde() {
        let call = ToolCall {
            id: "call_123".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        };
        let json = serde_json::to_string(&call).expect("serialize");
        let back: ToolCall = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, call);
    }

    #[rstest::rstest]
    fn tool_result_roundtrips_through_serde() {
        let result = ToolResult {
            tool_call_id: "call_123".to_owned(),
            name: "echo".to_owned(),
            content: "hi".to_owned(),
            success: true,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let back: ToolResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, result);
    }
}
