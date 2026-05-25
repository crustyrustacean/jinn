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
    let _result = super::builtin_task_incomplete::execute(make_call("missing tests"), ctx).await;

    // Then is_attached is still true.
    let guard = state.read();
    let session = guard.session(&judge_id);
    assert!(
        session.judge().as_ref().expect("has judge meta").is_attached,
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
    let _result = super::builtin_task_incomplete::execute(make_call("missing tests"), ctx).await;

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
    let _result = super::builtin_task_incomplete::execute(make_call("coverage is below threshold"), ctx).await;

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
    let result = super::builtin_task_incomplete::execute(make_call("reason"), ctx).await;

    // Then the result is an error.
    assert!(!result.success);
    assert!(result.content.contains("only be used in judge sessions"));
}
