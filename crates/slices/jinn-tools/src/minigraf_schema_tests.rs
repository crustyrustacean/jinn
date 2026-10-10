//! The advertised-schema strip: injected arguments never reach the model.
//!
//! Split from the dispatch tests so the red window is observable per filter.

use jinn_core_types::tool_types::ToolDefinition;

#[rstest::rstest]
#[test]
fn activate_tool_schema_hides_the_injected_arguments() {
    // Given a minigraf activate-tool definition advertising created_by.
    let def = ToolDefinition {
        name: "mcp__minigraf__instantiate_graph".to_owned(),
        description: String::new(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "template": {"type": "string"},
                "name": {"type": "string"},
                "created_by": {"type": "string"}
            },
            "required": ["template", "name", "created_by"]
        }),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        server_tool_type: None,
    };

    // When the orchestrator strips the injected arguments.
    let stripped = crate::orchestrator::strip_injected_args(def);

    // Then created_by is gone from properties and required, and the real
    // parameters survive.
    assert!(
        stripped
            .parameters
            .get("properties")
            .expect("props")
            .get("created_by")
            .is_none(),
        "created_by must not be advertised"
    );
    assert!(
        stripped
            .parameters
            .get("required")
            .expect("required")
            .as_array()
            .expect("arr")
            .iter()
            .all(|v| v.as_str() != Some("created_by")),
        "required must not name created_by"
    );
}
