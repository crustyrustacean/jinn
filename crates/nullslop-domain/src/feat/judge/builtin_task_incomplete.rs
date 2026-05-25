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

//! `task_incomplete` built-in tool — report that the origin session's task is NOT complete.

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::feat::tools_actor::BoxedToolFuture;
use crate::protocol::Event;

use super::protocol::{JudgeVerdict, Verdict};

/// Returns the tool definition for `task_incomplete`.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "task_incomplete".to_owned(),
        description: "Report that the origin session's task is NOT complete. \
            Provide a summary of what is missing or incorrect."
            .to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "A summary of what is incomplete or incorrect."
                }
            },
            "required": ["summary"]
        }),
    }
}

/// Executes the `task_incomplete` tool.
///
/// Does NOT change `is_attached` (it stays `true`). Emits a `JudgeVerdict(Fail(summary))` event.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let args: serde_json::Value = serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
        let summary = args
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("No summary provided.");

        let Some(state) = ctx.state else {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "Error: task_incomplete requires application state.".to_owned(),
                success: false,
                full_content: None,
                truncation: None,
            };
        };
        let Some(session_id) = ctx.session_id else {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "Error: task_incomplete requires a session ID.".to_owned(),
                success: false,
                full_content: None,
                truncation: None,
            };
        };

        // Read judge metadata (do NOT change is_attached).
        let (origin_id, judge_name) = {
            let state = state.read();
            let session = state.session(&session_id);
            let Some(judge_meta) = session.judge() else {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: "Error: task_incomplete can only be used in judge sessions."
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

        // Emit JudgeVerdict(Fail) event.
        if let Some(sink) = &ctx.sink {
            let event = Event::JudgeVerdict(JudgeVerdict {
                judge_session_id: session_id,
                origin_session_id: origin_id,
                judge_name,
                verdict: Verdict::Fail(summary.to_owned()),
            });
            if let Err(e) = sink.send_event(event) {
                tracing::warn!(err = ?e, "task_incomplete: failed to emit JudgeVerdict event");
            }
        }

        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: "Task marked as incomplete. Summary recorded.".to_owned(),
            success: true,
            full_content: None,
            truncation: None,
        }
    })
}
