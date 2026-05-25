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

//! `session_query` built-in tool — search the origin session's message history.

use std::fmt::Write;

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::feat::tools_actor::BoxedToolFuture;
use crate::protocol::ChatEntry;

/// Returns the tool definition for `session_query`.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "session_query".to_owned(),
        description: "Search the origin session's message history for relevant entries. \
            Use this to inspect what the agent has done so far."
            .to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query to match against session entries."
                }
            },
            "required": ["query"]
        }),
    }
}

/// Executes the `session_query` tool.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let args: serde_json::Value = serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if query.is_empty() {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "Error: query parameter is required.".to_owned(),
                success: false,
                full_content: None,
                truncation: None,
            };
        }

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
                content: "Error: session_query can only be used in judge sessions.".to_owned(),
                success: false,
                full_content: None,
                truncation: None,
            };
        };

        let origin_id = &judge_meta.origin_session;

        // Load origin session's history.
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
            .collect();

        // Simple substring matching for query.
        let scored: Vec<&ChatEntry> = entries
            .iter()
            .filter(|e| e.text().to_lowercase().contains(&query.to_lowercase()))
            .copied()
            .collect();

        if scored.is_empty() {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "No matching entries found.".to_owned(),
                success: true,
                full_content: None,
                truncation: None,
            };
        }

        // Format top results (limit to 20).
        let mut output = String::new();
        for entry in scored.iter().take(20) {
            let role = match entry.kind {
                crate::protocol::ChatEntryKind::System { .. } => "system",
                crate::protocol::ChatEntryKind::User { .. } => "user",
                crate::protocol::ChatEntryKind::Assistant { .. } => "assistant",
                crate::protocol::ChatEntryKind::ToolResult { .. } => "tool",
                _ => "other",
            };
            let text = entry.text();
            // Truncate individual entries to ~500 chars for readability.
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
        content: "Error: session_query requires application state.".to_owned(),
        success: false,
        full_content: None,
        truncation: None,
    }
}

fn error_no_session(call: &ToolCall) -> ToolResult {
    ToolResult {
        tool_call_id: call.id.clone(),
        name: call.name.clone(),
        content: "Error: session_query requires a session ID.".to_owned(),
        success: false,
        full_content: None,
        truncation: None,
    }
}
