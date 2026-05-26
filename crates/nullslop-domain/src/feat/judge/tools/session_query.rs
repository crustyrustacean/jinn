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

use crate::feat::tools_actor::BoxedToolFuture;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
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
        let args: serde_json::Value =
            serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");

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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::judge::JudgeMeta;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext};
    use crate::protocol::{ChatEntry, SessionId};

    fn make_context(session_id: SessionId, state: State) -> ToolContext {
        ToolContext {
            cwd: std::path::PathBuf::from("/tmp"),
            timeout: None,
            state: Some(state),
            session_id: Some(session_id),
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        }
    }

    fn make_call(query: &str) -> ToolCall {
        ToolCall {
            id: "test-call".to_owned(),
            name: "session_query".to_owned(),
            arguments: format!(r#"{{"query": "{query}"}}"#),
        }
    }

    fn setup_judge_with_origin_history() -> (State, SessionId, SessionId) {
        let mut state = AppState::default();
        let origin_id = state.session.active_session_id().clone();

        // Push some entries onto the origin session.
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("implement the login page"));
        state.active_session_mut().push_entry(ChatEntry::assistant(
            "I have implemented the login page with form validation.",
        ));

        // Create a judge session targeting this origin.
        let mut judge_session = ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "test-judge".to_owned(),
        });
        state.session.insert(judge_session);
        (State::new(state), judge_id, origin_id)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_query_returns_matching_entries() {
        // Given a judge session with origin that has known entries.
        let (state, judge_id, _origin_id) = setup_judge_with_origin_history();
        let ctx = make_context(judge_id, state);

        // When querying for "login".
        let result = super::execute(make_call("login"), ctx).await;

        // Then matching entries are returned.
        assert!(result.success, "session_query should succeed");
        assert!(
            result.content.contains("login"),
            "result should contain 'login': {}",
            result.content
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_query_returns_no_matches_for_nonexistent_text() {
        // Given a judge session with origin that has known entries.
        let (state, judge_id, _origin_id) = setup_judge_with_origin_history();
        let ctx = make_context(judge_id, state);

        // When querying for something not in the history.
        let result = super::execute(make_call("nonexistent_xyzzy"), ctx).await;

        // Then no matches found.
        assert!(
            result.success,
            "session_query should succeed even with no matches"
        );
        assert!(
            result.content.contains("No matching entries found"),
            "should report no matches: {}",
            result.content
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_query_errors_on_non_judge_session() {
        // Given a non-judge session.
        let state = State::new(AppState::default());
        let session_id = state.read().session.active_session_id().clone();
        let ctx = make_context(session_id, state);

        // When executing session_query.
        let result = super::execute(make_call("anything"), ctx).await;

        // Then the result is an error.
        assert!(!result.success);
        assert!(result.content.contains("only be used in judge sessions"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_query_errors_on_empty_query() {
        // Given a judge session.
        let (state, judge_id, _origin_id) = setup_judge_with_origin_history();
        let ctx = make_context(judge_id, state);

        // When querying with empty string.
        let call = ToolCall {
            id: "test-call".to_owned(),
            name: "session_query".to_owned(),
            arguments: r#"{"query": ""}"#.to_owned(),
        };
        let result = super::execute(call, ctx).await;

        // Then the result is an error.
        assert!(!result.success);
        assert!(result.content.contains("query parameter is required"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_query_case_insensitive() {
        // Given a judge session with origin that has "Login" in entries.
        let (state, judge_id, _origin_id) = setup_judge_with_origin_history();
        let ctx = make_context(judge_id, state);

        // When querying for "LOGIN" (uppercase).
        let result = super::execute(make_call("LOGIN"), ctx).await;

        // Then matching entries are still found (case-insensitive).
        assert!(result.success, "session_query should succeed");
        assert!(
            result.content.contains("login"),
            "case-insensitive match should work: {}",
            result.content
        );
    }
}
