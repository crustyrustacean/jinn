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

//! `set_list` built-in tool - replaces the entire task list in one call.

use crate::feat::tools_actor::BoxedToolFuture;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

/// Returns the tool definition for `set_list`.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "todo_set_list".to_owned(),
        description: "Replace the entire task list with a new one. \
            Accepts an ordered list of phases, each containing an ordered list of task \
            descriptions. All existing phases and tasks are discarded. All new tasks are \
            created with Pending status. Use this when you have a complete plan ready \
            to materialize."
            .to_owned(),
        prompt_snippet: Some("Create a new task list".to_owned()),
        prompt_guidelines: vec![
            "Provide the full plan - all phases and tasks - in a single call. \
             Existing phases and tasks are replaced entirely."
                .to_owned(),
            "Each phase must have a description. Tasks within a phase are optional.".to_owned(),
            "To preserve existing phases, read the current list first and include them.".to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "phases": {
                    "type": "array",
                    "description": "Ordered list of phases. Each phase has a description and an optional list of task descriptions.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "description": {
                                "type": "string",
                                "description": "Name of the phase (e.g., 'Research', 'Build', 'Test')"
                            },
                            "tasks": {
                                "type": "array",
                                "description": "Ordered list of task descriptions for this phase.",
                                "items": { "type": "string" }
                            }
                        },
                        "required": ["description"],
                        "additionalProperties": false
                    },
                    "minItems": 1
                }
            },
            "required": ["phases"],
            "additionalProperties": false
        }),
        server_tool_type: None,
    }
}

/// Executes the `set_list` tool.
///
/// # Panics
///
/// Does not panic under normal operation. Panics indicate a bug.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let Some(state) = ctx.state else {
            return tool_error(call, "no application state available");
        };
        let Some(session_id) = ctx.session_id else {
            return tool_error(call, "no session ID available");
        };

        let args: serde_json::Value =
            serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);

        let Some(phases_val) = args.get("phases") else {
            return tool_error(call, "missing 'phases' argument");
        };

        let Some(phases_arr) = phases_val.as_array() else {
            return tool_error(call, "'phases' must be an array");
        };

        if phases_arr.is_empty() {
            return tool_error(call, "'phases' must not be empty");
        }

        // Parse into (description, task_descriptions) tuples.
        let mut phase_data: Vec<(String, Vec<String>)> = Vec::new();
        for (i, phase_val) in phases_arr.iter().enumerate() {
            let desc = match phase_val.get("description").and_then(|v| v.as_str()) {
                Some(s) => s.to_owned(),
                None => {
                    return tool_error(
                        call,
                        &format!("phase at index {i} is missing 'description'"),
                    );
                }
            };

            let tasks: Vec<String> = phase_val
                .get("tasks")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            phase_data.push((desc, tasks));
        }

        let result = {
            let mut w = state.write();
            let session = w.session_mut(&session_id);
            let list = session.task_list_mut();
            match list.set_from_descriptions(phase_data) {
                Ok(()) => {
                    let rendered = list.render_text();
                    Ok(format!("Task list replaced.\n\n{rendered}"))
                }
                Err(e) => Err(format!("Error: {e}")),
            }
        };

        match result {
            Ok(content) => {
                if let Some(sink) = &ctx.sink {
                    let _ = sink.send_event(crate::protocol::Event::TaskListUpdated(
                        crate::feat::session::protocol::task_list_updated::TaskListUpdated {
                            session_id: session_id.clone(),
                        },
                    ));
                }
                ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content,
                    success: true,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                }
            }
            Err(content) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content,
                success: false,
                full_content: None,
                truncation: None,
                pin_position: None,
            },
        }
    })
}

fn tool_error(call: ToolCall, msg: &str) -> ToolResult {
    ToolResult {
        tool_call_id: call.id,
        name: call.name,
        content: format!("Error: {msg}"),
        success: false,
        full_content: None,
        truncation: None,
        pin_position: None,
    }
}

#[cfg(test)]
mod tests {
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::todo_list::TaskPosition;
    use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext};
    use crate::protocol::SessionId;

    use super::*;

    fn make_context(state: Option<State>, session_id: Option<SessionId>) -> ToolContext {
        ToolContext {
            cwd: std::path::PathBuf::from("."),
            timeout: None,
            bash_default_timeout: None,
            state,
            session_id,
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/bash".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        }
    }

    fn setup_with_existing_list() -> (State, SessionId) {
        let app = AppState::default();
        let state = State::new(app);
        let session_id = {
            let r = state.read();
            r.session.active_session_id().clone()
        };
        {
            let mut w = state.write();
            let session = w.session_mut(&session_id);
            let pid = session.task_list_mut().add_phase("Old Phase");
            session
                .task_list_mut()
                .add_task(&pid, "Old task", TaskPosition::End)
                .unwrap();
        };
        (state, session_id)
    }

    #[test]
    fn set_list_replaces_entire_list() {
        let (state, session_id) = setup_with_existing_list();
        let call = ToolCall {
            id: "call-1".to_owned(),
            name: "todo_set_list".to_owned(),
            arguments: serde_json::json!({
                "phases": [
                    { "description": "Research", "tasks": ["Read docs", "Call API"] },
                    { "description": "Build", "tasks": ["Write code"] },
                    { "description": "Deploy" }
                ]
            })
            .to_string(),
        };
        let ctx = make_context(Some(state), Some(session_id));
        let result = execute(call, ctx);
        let result = futures::executor::block_on(result);
        assert!(result.success, "expected success: {:?}", result.content);
        assert!(result.content.contains("Task list replaced"));
        assert!(result.content.contains("Research"));
        assert!(result.content.contains("Build"));
        assert!(result.content.contains("Deploy"));
        assert!(result.content.contains("Read docs"));
        assert!(result.content.contains("Write code"));
        assert!(!result.content.contains("Old Phase"));
        assert!(!result.content.contains("Old task"));
    }

    #[test]
    fn set_list_with_empty_tasks() {
        let (state, session_id) = setup_with_existing_list();
        let call = ToolCall {
            id: "call-1".to_owned(),
            name: "todo_set_list".to_owned(),
            arguments: serde_json::json!({
                "phases": [
                    { "description": "Planning" }
                ]
            })
            .to_string(),
        };
        let ctx = make_context(Some(state), Some(session_id));
        let result = execute(call, ctx);
        let result = futures::executor::block_on(result);
        assert!(result.success, "expected success: {:?}", result.content);
        assert!(result.content.contains("(no tasks)"));
    }

    #[test]
    fn set_list_errors_on_empty_phases() {
        let (state, session_id) = setup_with_existing_list();
        let call = ToolCall {
            id: "call-1".to_owned(),
            name: "todo_set_list".to_owned(),
            arguments: serde_json::json!({ "phases": [] }).to_string(),
        };
        let ctx = make_context(Some(state), Some(session_id));
        let result = execute(call, ctx);
        let result = futures::executor::block_on(result);
        assert!(!result.success);
        assert!(
            result.content.contains("must not be empty"),
            "expected empty error, got: {:?}",
            result.content
        );
    }

    #[test]
    fn set_list_errors_on_missing_phase_description() {
        let (state, session_id) = setup_with_existing_list();
        let call = ToolCall {
            id: "call-1".to_owned(),
            name: "todo_set_list".to_owned(),
            arguments: serde_json::json!({
                "phases": [
                    { "tasks": ["Do stuff"] }
                ]
            })
            .to_string(),
        };
        let ctx = make_context(Some(state), Some(session_id));
        let result = execute(call, ctx);
        let result = futures::executor::block_on(result);
        assert!(!result.success);
        assert!(
            result.content.contains("missing 'description'"),
            "expected missing description error, got: {:?}",
            result.content
        );
    }

    #[test]
    fn set_list_requires_state() {
        let call = ToolCall {
            id: "call-1".to_owned(),
            name: "todo_set_list".to_owned(),
            arguments: serde_json::json!({
                "phases": [{ "description": "Test" }]
            })
            .to_string(),
        };
        let ctx = make_context(None, Some(SessionId::new()));
        let result = execute(call, ctx);
        let result = futures::executor::block_on(result);
        assert!(!result.success);
        assert!(result.content.contains("no application state"));
    }
}
