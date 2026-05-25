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
    state
        .active_session_mut()
        .push_entry(ChatEntry::assistant("I have implemented the login page with form validation."));

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
    let result = super::builtin_session_query::execute(make_call("login"), ctx).await;

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
    let result = super::builtin_session_query::execute(make_call("nonexistent_xyzzy"), ctx).await;

    // Then no matches found.
    assert!(result.success, "session_query should succeed even with no matches");
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
    let result = super::builtin_session_query::execute(make_call("anything"), ctx).await;

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
    let result = super::builtin_session_query::execute(call, ctx).await;

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
    let result = super::builtin_session_query::execute(make_call("LOGIN"), ctx).await;

    // Then matching entries are still found (case-insensitive).
    assert!(result.success, "session_query should succeed");
    assert!(
        result.content.contains("login"),
        "case-insensitive match should work: {}",
        result.content
    );
}
