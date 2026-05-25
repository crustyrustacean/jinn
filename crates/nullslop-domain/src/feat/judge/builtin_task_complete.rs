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

//! `task_complete` built-in tool — mark the origin session's task as passed.

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::feat::tools_actor::BoxedToolFuture;
use crate::protocol::Event;

use super::protocol::{JudgeVerdict, Verdict};

/// Returns the tool definition for `task_complete`.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "task_complete".to_owned(),
        description: "Mark the origin session's task as successfully completed. \
            Call this when the agent's work meets all acceptance criteria."
            .to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    }
}

/// Executes the `task_complete` tool.
///
/// Sets `is_attached = false` on the judge session and emits a `JudgeVerdict(Pass)` event.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let Some(state) = ctx.state else {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "Error: task_complete requires application state.".to_owned(),
                success: false,
                full_content: None,
                truncation: None,
            };
        };
        let Some(session_id) = ctx.session_id else {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "Error: task_complete requires a session ID.".to_owned(),
                success: false,
                full_content: None,
                truncation: None,
            };
        };

        // Read judge metadata.
        let (origin_id, judge_name) = {
            let state = state.read();
            let session = state.session(&session_id);
            let Some(judge_meta) = session.judge() else {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: "Error: task_complete can only be used in judge sessions."
                        .to_owned(),
                    success: false,
                    full_content: None,
                    truncation: None,
                };
            };
            (
                judge_meta.origin_session.clone(),
                judge_meta.judge_name.clone(),
            )
        };

        // Set is_attached = false.
        {
            let mut state = state.write();
            let session = state.session_mut(&session_id);
            session.set_judge_attached(false);
        }

        // Emit JudgeVerdict(Pass) event.
        if let Some(sink) = &ctx.sink {
            let event = Event::JudgeVerdict(JudgeVerdict {
                judge_session_id: session_id,
                origin_session_id: origin_id,
                judge_name,
                verdict: Verdict::Pass,
            });
            if let Err(e) = sink.send_event(event) {
                tracing::warn!(err = ?e, "task_complete: failed to emit JudgeVerdict event");
            }
        }

        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: "Task marked as complete. Evaluation passed.".to_owned(),
            success: true,
            full_content: None,
            truncation: None,
        }
    })
}
