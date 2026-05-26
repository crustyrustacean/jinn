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

use crate::feat::tools_actor::BoxedToolFuture;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::Event;

use super::super::protocol::{JudgeVerdict, Verdict};

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
        let args: serde_json::Value =
            serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
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

        // Disable tool loop so the session stops after this tool batch.
        {
            let mut state = state.write();
            let session = state.session_mut(&session_id);
            session.set_tool_loop_disabled();
        }

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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use std::sync::Arc;

    use crate::common::actor::RecordingSink;
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::judge::{JudgeMeta, Verdict};
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext};
    use crate::protocol::{Event, SessionId};

    fn make_context(session_id: SessionId, state: State) -> ToolContext {
        ToolContext {
            cwd: std::path::PathBuf::from("/tmp"),
            timeout: None,
            state: Some(state),
            session_id: Some(session_id),
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: Some(Arc::new(RecordingSink::new())),
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        }
    }

    fn make_call(summary: &str) -> ToolCall {
        ToolCall {
            id: "test-call".to_owned(),
            name: "task_incomplete".to_owned(),
            arguments: format!(r#"{{"summary": "{summary}"}}"#),
        }
    }

    fn setup_judge_session() -> (State, SessionId, SessionId) {
        let mut state = AppState::default();
        let origin_id = state.session.active_session_id().clone();
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
    async fn task_incomplete_leaves_is_attached_true() {
        // Given a judge session with is_attached = true.
        let (state, judge_id, _origin_id) = setup_judge_session();
        let ctx = make_context(judge_id.clone(), state.clone());

        // When executing task_incomplete.
        let _result = super::execute(make_call("missing tests"), ctx).await;

        // Then is_attached is still true.
        let guard = state.read();
        let session = guard.session(&judge_id);
        assert!(
            session
                .judge()
                .as_ref()
                .expect("has judge meta")
                .is_attached,
            "is_attached should still be true after task_incomplete"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn task_incomplete_emits_verdict_fail() {
        // Given a judge session.
        let (state, judge_id, origin_id) = setup_judge_session();
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = make_context(judge_id.clone(), state.clone());
        ctx.sink = Some(sink.clone());

        // When executing task_incomplete with a summary.
        let _result = super::execute(make_call("missing tests"), ctx).await;

        // Then a JudgeVerdict(Fail) event was emitted.
        let events = sink.events();
        let verdict = events
            .iter()
            .find_map(|e| match e {
                Event::JudgeVerdict(v) => Some(v.clone()),
                _ => None,
            })
            .expect("expected JudgeVerdict event");
        assert_eq!(verdict.judge_session_id, judge_id);
        assert_eq!(verdict.origin_session_id, origin_id);
        assert!(matches!(verdict.verdict, Verdict::Fail(_)));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn task_incomplete_includes_summary_in_verdict() {
        // Given a judge session.
        let (state, judge_id, _origin_id) = setup_judge_session();
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = make_context(judge_id.clone(), state.clone());
        ctx.sink = Some(sink.clone());

        // When executing task_incomplete with a specific summary.
        let _result =
            super::execute(make_call("coverage is below threshold"), ctx).await;

        // Then the verdict contains the summary.
        let events = sink.events();
        let verdict = events
            .iter()
            .find_map(|e| match e {
                Event::JudgeVerdict(v) => Some(v.clone()),
                _ => None,
            })
            .expect("expected JudgeVerdict event");
        if let Verdict::Fail(ref summary) = verdict.verdict {
            assert_eq!(summary, "coverage is below threshold");
        } else {
            panic!("expected Fail verdict");
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn task_incomplete_errors_on_non_judge_session() {
        // Given a non-judge session.
        let state = State::new(AppState::default());
        let session_id = state.read().session.active_session_id().clone();
        let ctx = make_context(session_id, state);

        // When executing task_incomplete.
        let result = super::execute(make_call("reason"), ctx).await;

        // Then the result is an error.
        assert!(!result.success);
        assert!(result.content.contains("only be used in judge sessions"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn task_incomplete_sets_tool_loop_disabled() {
        // Given a judge session.
        let (state, judge_id, _origin_id) = setup_judge_session();
        let ctx = make_context(judge_id.clone(), state.clone());

        // When executing task_incomplete.
        let _result = super::execute(make_call("missing"), ctx).await;

        // Then the tool loop is disabled.
        let mut guard = state.write();
        let session = guard.session_mut(&judge_id);
        assert!(
            session.take_tool_loop_disabled(),
            "tool_loop_disabled should be true after task_incomplete"
        );
        // And it clears on read.
        assert!(
            !session.take_tool_loop_disabled(),
            "tool_loop_disabled should be cleared after take"
        );
    }
}
