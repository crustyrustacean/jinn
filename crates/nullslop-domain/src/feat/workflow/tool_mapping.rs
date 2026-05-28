//! Conversion from workflow-level [`ToolSchema`] to domain-level [`ToolDefinition`].
//!
//! This module bridges the lightweight tool description type used in workflow
//! configurations (`nullslop-workflow::tool_schema::ToolSchema`) with the full
//! provider tool definition type (`nullslop-provider::ToolDefinition`). The
//! mapping fills in `prompt_snippet` and `prompt_guidelines` with defaults since
//! workflow-specified tools don't need those fields.

use crate::protocol::ToolDefinition;
use nullslop_workflow::tool_schema::ToolSchema;

/// Convert a workflow [`ToolSchema`] to a provider [`ToolDefinition`].
///
/// The `prompt_snippet` is set to the tool's description and `prompt_guidelines`
/// is left empty, since workflow-defined tools don't customize those fields.
#[must_use]
pub fn tool_schema_to_definition(schema: &ToolSchema) -> ToolDefinition {
    ToolDefinition {
        name: schema.name.clone(),
        description: schema.description.clone(),
        parameters: schema.parameters.clone(),
        prompt_snippet: Some(schema.description.clone()),
        prompt_guidelines: vec![],
        server_tool_type: None,
    }
}

/// Convert a batch of workflow [`ToolSchema`]s to provider [`ToolDefinition`]s.
#[must_use]
pub fn tool_schemas_to_definitions(schemas: &[ToolSchema]) -> Vec<ToolDefinition> {
    schemas.iter().map(tool_schema_to_definition).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_single_tool_schema() {
        let schema = ToolSchema {
            name: "get_weather".into(),
            description: "Get current weather".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        let def = tool_schema_to_definition(&schema);
        assert_eq!(def.name, "get_weather");
        assert_eq!(def.description, "Get current weather");
        assert_eq!(def.parameters, serde_json::json!({"type": "object"}));
        assert_eq!(def.prompt_snippet.as_deref(), Some("Get current weather"));
        assert!(def.prompt_guidelines.is_empty());
    }

    #[test]
    fn maps_batch() {
        let schemas = vec![
            ToolSchema {
                name: "a".into(),
                description: "Tool A".into(),
                parameters: serde_json::json!({}),
            },
            ToolSchema {
                name: "b".into(),
                description: "Tool B".into(),
                parameters: serde_json::json!({}),
            },
        ];
        let defs = tool_schemas_to_definitions(&schemas);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "a");
        assert_eq!(defs[1].name, "b");
    }
}
