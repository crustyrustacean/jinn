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

//! `task_complete` built-in tool - mark the origin session's task as passed.

use crate::feat::tools_actor::BoxedToolFuture;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::Event;

use super::super::protocol::{JudgeVerdict, Verdict};

/// Returns the tool definition for `task_complete`.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "judge_task_complete".to_owned(),
        description: "Mark the origin session's task as successfully completed. \
            Call this when the agent's work meets all acceptance criteria."
            .to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {}
        }),
        server_tool_type: None,
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
                    content: "Error: task_complete can only be used in judge sessions.".to_owned(),
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

        // Set is_attached = false and disable tool loop.
        {
            let mut state = state.write();
            let session = state.session_mut(&session_id);
            session.set_judge_attached(false);
            session.set_tool_loop_disabled();
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

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

    fn make_call() -> ToolCall {
        ToolCall {
            id: "test-call".to_owned(),
            name: "judge_task_complete".to_owned(),
            arguments: "{}".to_owned(),
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
auto_reset: None,
});
        state.session.insert(judge_session);
        (State::new(state), judge_id, origin_id)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn task_complete_sets_is_attached_false() {
        // Given a judge session with is_attached = true.
        let (state, judge_id, _origin_id) = setup_judge_session();
        let ctx = make_context(judge_id.clone(), state.clone());

        // When executing task_complete.
        let _result = super::execute(make_call(), ctx).await;

        // Then is_attached is false.
        let guard = state.read();
        let session = guard.session(&judge_id);
        assert!(
            !session
                .judge()
                .as_ref()
                .expect("has judge meta")
                .is_attached,
            "is_attached should be false after task_complete"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn task_complete_emits_verdict_pass() {
        // Given a judge session.
        let (state, judge_id, origin_id) = setup_judge_session();
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = make_context(judge_id.clone(), state.clone());
        ctx.sink = Some(sink.clone());

        // When executing task_complete.
        let _result = super::execute(make_call(), ctx).await;

        // Then a JudgeVerdict(Pass) event was emitted.
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
        assert!(matches!(verdict.verdict, Verdict::Pass));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn task_complete_errors_on_non_judge_session() {
        // Given a non-judge session.
        let state = State::new(AppState::default());
        let session_id = state.read().session.active_session_id().clone();
        let ctx = make_context(session_id, state);

        // When executing task_complete.
        let result = super::execute(make_call(), ctx).await;

        // Then the result is an error.
        assert!(!result.success);
        assert!(result.content.contains("only be used in judge sessions"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn task_complete_sets_tool_loop_disabled() {
        // Given a judge session.
        let (state, judge_id, _origin_id) = setup_judge_session();
        let ctx = make_context(judge_id.clone(), state.clone());

        // When executing task_complete.
        let _result = super::execute(make_call(), ctx).await;

        // Then the tool loop is disabled.
        let mut guard = state.write();
        let session = guard.session_mut(&judge_id);
        assert!(
            session.take_tool_loop_disabled(),
            "tool_loop_disabled should be true after task_complete"
        );
        // And it clears on read.
        assert!(
            !session.take_tool_loop_disabled(),
            "tool_loop_disabled should be cleared after take"
        );
    }
}
