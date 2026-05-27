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
        let count = args.get("count").and_then(serde_json::Value::as_u64).unwrap_or(10) as usize;

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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::judge::JudgeMeta;
    use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolResult};
    use crate::protocol::{ChatEntry, SessionId};

    fn make_tool_context(state: &State, session_id: SessionId) -> ToolContext {
        ToolContext {
            cwd: std::path::PathBuf::new(),
            timeout: None,
            state: Some(state.clone()),
            session_id: Some(session_id),
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: String::new(),
            max_output_lines: None,
            max_output_bytes: None,
        }
    }

    fn make_call(args: &str) -> ToolCall {
        ToolCall {
            id: "test-call".to_owned(),
            name: "session_query_recent".to_owned(),
            arguments: args.to_owned(),
        }
    }

    fn run_tool(state: &State, session_id: SessionId, args: &str) -> ToolResult {
        let ctx = make_tool_context(state, session_id);
        let call = make_call(args);
        futures::executor::block_on(super::execute(call, ctx))
    }

    #[rstest::rstest]
    fn returns_recent_entries() {
        // Given an origin session with several entries and a judge session.
        let state = State::new(AppState::default());
        let origin_id = state.read().session.active_session_id().clone();

        {
            let mut guard = state.write();
            let origin = guard.session_mut(&origin_id);
            origin.push_entry(ChatEntry::user("first message"));
            origin.push_entry(ChatEntry::assistant("first response"));
            origin.push_entry(ChatEntry::user("second message"));
            origin.push_entry(ChatEntry::assistant("second response"));
        }

        // Create judge session.
        let mut judge_session = crate::feat::session::chat_session::ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id,
            is_attached: true,
            judge_name: "test-judge".to_owned(),
auto_reset: None,
});
        state.write().session.insert(judge_session);

        // When requesting 2 most recent entries.
        let result = run_tool(&state, judge_id, r#"{"count": 2}"#);

        // Then the result contains the last 2 entries.
        assert!(result.success);
        assert!(result.content.contains("second message"));
        assert!(result.content.contains("second response"));
        assert!(!result.content.contains("first message"));
        assert!(!result.content.contains("first response"));
    }

    #[rstest::rstest]
    fn defaults_to_10_entries() {
        // Given an origin session with 15 entries and a judge session.
        let state = State::new(AppState::default());
        let origin_id = state.read().session.active_session_id().clone();

        {
            let mut guard = state.write();
            let origin = guard.session_mut(&origin_id);
            for i in 0..15 {
                origin.push_entry(ChatEntry::user(format!("message {i}")));
            }
        }

        let mut judge_session = crate::feat::session::chat_session::ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id,
            is_attached: true,
            judge_name: "test-judge".to_owned(),
auto_reset: None,
});
        state.write().session.insert(judge_session);

        // When calling without count parameter.
        let result = run_tool(&state, judge_id, "{}");

        // Then it returns the last 10 entries (messages 5-14).
        assert!(result.success);
        assert!(result.content.contains("message 5"));
        assert!(result.content.contains("message 14"));
        assert!(!result.content.contains("message 4"));
    }

    #[rstest::rstest]
    fn errors_on_non_judge_session() {
        // Given a regular (non-judge) session.
        let state = State::new(AppState::default());
        let origin_id = state.read().session.active_session_id().clone();

        let result = run_tool(&state, origin_id, r#"{"count": 5}"#);

        // Then it errors.
        assert!(!result.success);
        assert!(result.content.contains("only be used in judge sessions"));
    }

    #[rstest::rstest]
    fn returns_empty_for_empty_origin() {
        // Given an origin session with no history.
        let state = State::new(AppState::default());
        let origin_id = state.read().session.active_session_id().clone();

        let mut judge_session = crate::feat::session::chat_session::ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id,
            is_attached: true,
            judge_name: "test-judge".to_owned(),
auto_reset: None,
});
        state.write().session.insert(judge_session);

        // When querying recent messages.
        let result = run_tool(&state, judge_id, r#"{"count": 5}"#);

        // Then it returns the empty message.
        assert!(result.success);
        assert!(result.content.contains("No entries found"));
    }

    #[rstest::rstest]
    fn session_query_recent_uses_correct_role_labels() {
        // Given a judge session with origin entries of struct-variant kinds.
        let state = State::new(AppState::default());
        let origin_id = state.read().session.active_session_id().clone();

        {
            let mut guard = state.write();
            let origin = guard.session_mut(&origin_id);
            origin.push_entry(ChatEntry::user("user_msg_xyzzy"));
            origin.push_entry(ChatEntry::assistant("assistant_msg_xyzzy"));
            origin.push_entry(ChatEntry::tool_result(
                "tr-1",
                "tool_name",
                "toolresult_msg_xyzzy",
                crate::feat::session::tool_result_status::ToolResultStatus::Success,
            ));
        }

        let mut judge_session = crate::feat::session::chat_session::ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id,
            is_attached: true,
            judge_name: "test-judge".to_owned(),
auto_reset: None,
});
        state.write().session.insert(judge_session);

        // When querying recent entries.
        let result = run_tool(&state, judge_id, r#"{\"count\": 3}"#);

        // Then each entry is labeled with its correct role.
        assert!(result.success);
        assert!(result.content.contains("[user]"), "should label user entries: {}", result.content);
        assert!(result.content.contains("[assistant]"), "should label assistant entries: {}", result.content);
        assert!(result.content.contains("[tool]"), "should label tool_result entries: {}", result.content);
        // None of these struct-variant entries should be labeled "other".
        assert!(!result.content.contains("[other]"), "no struct-variant entries should be labeled 'other': {}", result.content);
    }

    #[rstest::rstest]
    fn session_query_recent_truncates_at_500_chars() {
        // Given a judge session with origin entry whose text is > 500 chars.
        let state = State::new(AppState::default());
        let origin_id = state.read().session.active_session_id().clone();

        let long_text = format!("{}XYZ", "a".repeat(500));
        {
            let mut guard = state.write();
            let origin = guard.session_mut(&origin_id);
            origin.push_entry(ChatEntry::user(long_text));
        }

        let mut judge_session = crate::feat::session::chat_session::ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id,
            is_attached: true,
            judge_name: "test-judge".to_owned(),
auto_reset: None,
});
        state.write().session.insert(judge_session);

        // When querying.
        let result = run_tool(&state, judge_id, r#"{\"count\": 1}"#);

        // Then the output is truncated.
        assert!(result.success);
        assert!(result.content.contains("..."), "long entry should be truncated: {}", result.content);
        assert!(!result.content.contains("XYZ"), "text beyond 500 chars should be truncated: {}", result.content);
    }

    #[rstest::rstest]
    fn session_query_recent_does_not_truncate_at_exactly_500_chars() {
        // Given a judge session with origin entry whose text is exactly 500 chars.
        let state = State::new(AppState::default());
        let origin_id = state.read().session.active_session_id().clone();

        let exact_text = format!("{}END", "a".repeat(497)); // 500 chars total
        assert_eq!(exact_text.len(), 500);
        {
            let mut guard = state.write();
            let origin = guard.session_mut(&origin_id);
            origin.push_entry(ChatEntry::user(exact_text));
        }

        let mut judge_session = crate::feat::session::chat_session::ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id,
            is_attached: true,
            judge_name: "test-judge".to_owned(),
auto_reset: None,
});
        state.write().session.insert(judge_session);

        // When querying.
        let result = run_tool(&state, judge_id, r#"{\"count\": 1}"#);

        // Then the output is NOT truncated (text.len() > 500 is false for exactly 500).
        assert!(result.success);
        assert!(!result.content.contains("..."), "500-char text should not be truncated: {}", result.content);
        assert!(result.content.contains("END"), "full text should be present: {}", result.content);
    }
}
