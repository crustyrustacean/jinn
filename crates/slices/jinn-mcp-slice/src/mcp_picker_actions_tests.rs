// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Tests for the inspector's live tool projection.

#[rstest::rstest]
#[test]
fn tools_for_collects_only_the_named_servers_tools() {
    // Given tool definitions from this server, another server, and a builtin.
    let definitions = vec![
        tool_def("mcp__excalimate__create_scene", "Create a scene"),
        tool_def("mcp__excalimate__auto_animate", "Auto-animate"),
        tool_def("mcp__other__create_scene", "Other server"),
        tool_def("file_read", "A builtin"),
    ];

    // When projecting the tools for "excalimate".
    let tools = super::mcp_picker_actions::tools_for("excalimate", &definitions);

    // Then only excalimate's tools are collected, with prefixes stripped.
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].0, "create_scene");
    assert_eq!(tools[0].1, "Create a scene");
    assert_eq!(tools[1].0, "auto_animate");
}

#[rstest::rstest]
#[test]
fn tools_for_returns_empty_when_no_definition_matches() {
    // Given definitions with no matching prefix.
    let definitions = vec![tool_def("file_read", "builtin")];

    // When projecting the tools for an unknown server.
    let tools = super::mcp_picker_actions::tools_for("ghost", &definitions);

    // Then no tools are collected.
    assert!(tools.is_empty());
}

/// A minimal tool definition the projection tests only need a shape for.
fn tool_def(name: &str, description: &str) -> jinn_core_types::ToolDefinition {
    jinn_core_types::ToolDefinition {
        name: name.to_owned(),
        description: description.to_owned(),
        parameters: serde_json::Value::Object(serde_json::Map::new()),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        server_tool_type: None,
    }
}
