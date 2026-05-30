//! Lightweight tool schema type for workflow node definitions.
//!
//! [`ToolSchema`] is a minimal representation of an LLM tool definition
//! that can be specified in workflow configurations without pulling in
//! the heavy `jinn-provider` crate. The domain layer maps these to
//! full [`ToolDefinition`](jinn_provider::ToolDefinition) instances
//! at runtime.

use serde::{Deserialize, Serialize};

/// A lightweight description of a tool the LLM can invoke during a workflow step.
///
/// Contains just enough information to produce a provider-level tool definition:
/// a name, a human-readable description, and a JSON Schema for the parameters.
///
/// # Example
///
/// ```
/// use jinn_workflow::tool_schema::ToolSchema;
///
/// let schema = ToolSchema {
///     name: "get_weather".into(),
///     description: "Get current weather for a location".into(),
///     parameters: serde_json::json!({
///         "type": "object",
///         "properties": {
///             "location": { "type": "string" }
///         },
///         "required": ["location"]
///     }),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSchema {
    /// The unique name of the tool (e.g., "file_read").
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    pub parameters: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schema_round_trips_through_json() {
        let schema = ToolSchema {
            name: "test_tool".into(),
            description: "A test tool".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        let json = serde_json::to_string(&schema).unwrap();
        let back: ToolSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(schema, back);
    }
}
