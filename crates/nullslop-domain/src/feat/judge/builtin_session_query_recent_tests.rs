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

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::common::app_state::AppState;
use crate::common::state::State;
use crate::feat::judge::JudgeMeta;
use crate::feat::judge::builtin_session_query_recent;
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
    futures::executor::block_on(builtin_session_query_recent::execute(call, ctx))
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
    });
    state.write().session.insert(judge_session);

    // When querying recent messages.
    let result = run_tool(&state, judge_id, r#"{"count": 5}"#);

    // Then it returns the empty message.
    assert!(result.success);
    assert!(result.content.contains("No entries found"));
}
