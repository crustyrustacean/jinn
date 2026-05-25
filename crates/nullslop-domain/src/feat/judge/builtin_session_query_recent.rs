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

//! `session_query_recent` built-in tool — retrieve the N most recent messages from the origin session.

use std::fmt::Write;

use crate::feat::tools_actor::BoxedToolFuture;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::ChatEntry;

/// Returns the tool definition for `session_query_recent`.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "session_query_recent".to_owned(),
        description: "Retrieve the most recent messages from the origin session. \
            Returns the last N entries (user, assistant, tool results) in chronological order. \
            Use this to quickly review what the agent just did without searching."
            .to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "count": {
                    "type": "integer",
                    "description": "Number of most recent messages to return. Defaults to 10."
                }
            },
            "required": []
        }),
    }
}

/// Executes the `session_query_recent` tool.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let args: serde_json::Value =
            serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let count = count.clamp(1, 100);

        let Some(state) = ctx.state else {
            return error_no_state(&call);
        };
        let Some(session_id) = ctx.session_id else {
            return error_no_session(&call);
        };

        let state = state.read();
        let session = state.session(&session_id);
        let Some(judge_meta) = session.judge() else {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "Error: session_query_recent can only be used in judge sessions."
                    .to_owned(),
                success: false,
                full_content: None,
                truncation: None,
            };
        };

        let origin_id = &judge_meta.origin_session;

        let Some(origin_session) = state.session.get(origin_id) else {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("Error: origin session {origin_id} not found."),
                success: false,
                full_content: None,
                truncation: None,
            };
        };

        let entries: Vec<&ChatEntry> = origin_session
            .history()
            .iter()
            .filter(|e| e.is_in_context())
            .rev()
            .take(count)
            .collect();

        if entries.is_empty() {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "No entries found in origin session.".to_owned(),
                success: true,
                full_content: None,
                truncation: None,
            };
        }

        // Format in chronological order (oldest first).
        let mut output = String::new();
        for entry in entries.into_iter().rev() {
            let role = match entry.kind {
                crate::protocol::ChatEntryKind::System { .. } => "system",
                crate::protocol::ChatEntryKind::User { .. } => "user",
                crate::protocol::ChatEntryKind::Assistant { .. } => "assistant",
                crate::protocol::ChatEntryKind::ToolResult { .. } => "tool",
                _ => "other",
            };
            let text = entry.text();
            let display = if text.len() > 500 {
                format!("{}...", &text[..500])
            } else {
                text.clone()
            };
            let _ = write!(output, "[{role}] {display}\n\n");
        }

        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: output,
            success: true,
            full_content: None,
            truncation: None,
        }
    })
}

fn error_no_state(call: &ToolCall) -> ToolResult {
    ToolResult {
        tool_call_id: call.id.clone(),
        name: call.name.clone(),
        content: "Error: session_query_recent requires application state.".to_owned(),
        success: false,
        full_content: None,
        truncation: None,
    }
}

fn error_no_session(call: &ToolCall) -> ToolResult {
    ToolResult {
        tool_call_id: call.id.clone(),
        name: call.name.clone(),
        content: "Error: session_query_recent requires a session ID.".to_owned(),
        success: false,
        full_content: None,
        truncation: None,
    }
}
