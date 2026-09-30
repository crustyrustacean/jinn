//! The tool filter's runtime gate: a withheld tool is refused where the call is
//! routed, not only where the prompt is built.
//!
//! These drive the REAL delivery path — `ExecuteToolBatch` published on the
//! trouper bus, the spawned orchestrator resolving the tool, and
//! `ToolBatchCompleted` coming back — because the defect they guard is one of
//! reachability. A filter applied only to the assembled prompt looks correct in
//! every prompt-level test while the tool itself stays callable by name.
//!
//! The refusal is asserted on the observable result the model receives, not on
//! whether a task was spawned: the point is what the model is told.

use crate::orchestrator::{ToolOrchestratorActor, ToolOrchestratorActorDeps};
use jinn_core_types::tool_types::{ToolCall, ToolResult};
use jinn_core_types::{FilterMode, NameFilter, SessionId};
use jinn_kernel::common::bus::HarnessServices;
use jinn_kernel::common::state::State;
use jinn_testutil::bus_harness::{TestHarness, await_recorded};
use jinn_tools_msg::{ExecuteToolBatch, ToolBatchCompleted};
use std::time::Duration;

/// A state whose active session withholds `filter`'s worth of tools.
fn state_filtering(filter: NameFilter) -> (State, SessionId) {
    let state = State::new(jinn_kernel::AppState::default());
    let session_id = {
        let mut guard = state.write();
        let session = guard.active_session_mut();
        session.set_tool_filter(filter);
        session.session_id().clone()
    };
    (state, session_id)
}

/// Spawns the orchestrator over `state`, with `bash` as its only builtin.
async fn spawn(harness: &TestHarness, state: State) {
    ToolOrchestratorActor::spawn(
        harness.system(),
        ToolOrchestratorActorDeps {
            deps: harness.actor_deps().await,
            state,
            services: harness.services().await,
            builtin_filter: Some(vec!["bash".to_owned()]),
        },
    );
}

/// Publishes one `bash` call and returns the single result it produced.
async fn call_bash(harness: &TestHarness, session_id: &SessionId) -> ToolResult {
    let recorder = harness.spawn_recorder::<ToolBatchCompleted>().await;
    harness
        .publish(ExecuteToolBatch {
            session_id: session_id.clone(),
            tool_calls: vec![ToolCall {
                id: "tc-1".to_owned(),
                name: "bash".to_owned(),
                arguments: r#"{"command":"echo should-not-run"}"#.to_owned(),
            }],
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;
    let batches = await_recorded::<ToolBatchCompleted>(&recorder, 1, Duration::from_secs(10)).await;
    let results = batches
        .into_iter()
        .flat_map(|batch| batch.results)
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1, "expected exactly one tool result");
    results.into_iter().next().expect("one result")
}

#[rstest::rstest]
#[tokio::test]
async fn a_filtered_builtin_is_refused_and_named() {
    // Given a session whose filter withholds bash.
    let (state, session_id) = state_filtering(NameFilter::deny(["bash".to_owned()]));
    let harness = TestHarness::new().await;
    spawn(&harness, state).await;

    // When the model calls bash.
    let result = call_bash(&harness, &session_id).await;

    // Then the call fails and the message names the tool, so the model can
    // stop rather than retry or hunt for an alias.
    assert!(!result.success, "a withheld tool must not succeed");
    assert!(
        result.content.contains("bash"),
        "the refusal must name the tool; got: {:?}",
        result.content
    );
}

#[rstest::rstest]
#[tokio::test]
async fn a_filtered_tool_is_reported_as_withheld_not_unknown() {
    // Given a session whose filter withholds bash.
    let (state, session_id) = state_filtering(NameFilter::deny(["bash".to_owned()]));
    let harness = TestHarness::new().await;
    spawn(&harness, state).await;

    // When the model calls bash.
    let result = call_bash(&harness, &session_id).await;

    // Then it is not reported as unknown. The tool exists and is registered;
    // calling it unknown would be false and would point the model at a
    // spelling problem instead of a scope one.
    assert!(
        !result.content.contains("unknown tool"),
        "a withheld tool must not be reported as unknown; got: {:?}",
        result.content
    );
}

#[rstest::rstest]
#[tokio::test]
async fn an_unfiltered_builtin_still_dispatches() {
    // Given a session with no filter configured.
    let (state, session_id) = state_filtering(NameFilter::default());
    let harness = TestHarness::new().await;
    spawn(&harness, state).await;

    // When the model calls bash.
    let result = call_bash(&harness, &session_id).await;

    // Then it runs. The gate must not break the happy path — a filter that
    // withholds everything would pass every refusal test above.
    assert!(result.success, "an unfiltered tool must still run");
    assert!(
        result.content.contains("should-not-run"),
        "the tool must actually have executed; got: {:?}",
        result.content
    );
}

#[rstest::rstest]
#[tokio::test]
async fn an_allow_filter_refuses_a_tool_it_does_not_name() {
    // Given a session permitted exactly one other tool.
    let (state, session_id) = state_filtering(NameFilter {
        mode: FilterMode::Allow,
        names: ["read".to_owned()].into_iter().collect(),
    });
    let harness = TestHarness::new().await;
    spawn(&harness, state).await;

    // When the model calls bash.
    let result = call_bash(&harness, &session_id).await;

    // Then the call is refused — the case a blocklist could not express,
    // since there is no bash entry to remove.
    assert!(!result.success, "an unlisted tool must be withheld");
    assert!(
        result.content.contains("withheld"),
        "the refusal must say the filter withheld it; got: {:?}",
        result.content
    );
}

#[rstest::rstest]
#[tokio::test]
async fn an_allow_filter_still_admits_the_tool_it_names() {
    // Given a session permitted exactly bash.
    let (state, session_id) = state_filtering(NameFilter {
        mode: FilterMode::Allow,
        names: ["bash".to_owned()].into_iter().collect(),
    });
    let harness = TestHarness::new().await;
    spawn(&harness, state).await;

    // When the model calls bash.
    let result = call_bash(&harness, &session_id).await;

    // Then it runs. A filter that withholds by omission must not also
    // withhold what it names.
    assert!(
        result.success,
        "a named tool must still run; got: {:?}",
        result.content
    );
}

#[rstest::rstest]
#[tokio::test]
async fn a_glob_withholding_a_whole_mcp_server_refuses_its_tools() {
    // Given a session withholding every tool from one MCP server.
    let (state, session_id) = state_filtering(NameFilter::deny(["mcp__github__*".to_owned()]));
    let harness = TestHarness::new().await;
    spawn(&harness, state).await;

    // When the model calls a tool from that server.
    let recorder = harness.spawn_recorder::<ToolBatchCompleted>().await;
    harness
        .publish(ExecuteToolBatch {
            session_id: session_id.clone(),
            tool_calls: vec![ToolCall {
                id: "tc-mcp".to_owned(),
                name: "mcp__github__create_pr".to_owned(),
                arguments: "{}".to_owned(),
            }],
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;
    let batches = await_recorded::<ToolBatchCompleted>(&recorder, 1, Duration::from_secs(10)).await;
    let results = batches
        .into_iter()
        .flat_map(|batch| batch.results)
        .collect::<Vec<_>>();

    // Then the refusal is the filter's, not "unknown tool" — which is what
    // this call would otherwise produce, since no MCP server is registered
    // in the test. The gate runs before registration lookup, so a withheld
    // tool reports as withheld whatever its registration state.
    assert_eq!(results.len(), 1, "expected exactly one tool result");
    assert!(
        results[0].content.contains("withheld"),
        "a globbed MCP tool must be refused by the filter; got: {:?}",
        results[0].content
    );
}

/// The two filters are separate gates over separate resources, and neither can
/// widen the other. A skill that is withheld cannot be the reason a withheld
/// tool runs — and a withheld tool is refused whatever the skill filter says.
#[rstest::rstest]
#[tokio::test]
async fn a_withheld_skill_does_not_admit_a_withheld_tool() {
    // Given a session withholding both a skill and a tool.
    let (state, session_id) = state_filtering(NameFilter::deny(["bash".to_owned()]));
    {
        let mut guard = state.write();
        guard
            .active_session_mut()
            .set_skill_filter(NameFilter::deny(["web-coder".to_owned()]));
    }
    let harness = TestHarness::new().await;
    spawn(&harness, state).await;

    // When the model calls the withheld tool.
    let result = call_bash(&harness, &session_id).await;

    // Then it is refused. The skill gate is not a route around the tool gate.
    assert!(!result.success, "the tool gate must still refuse");
}

/// A session the orchestrator has not seen has no filter to consult. Defaulting
/// the gate to "withheld" in that case would refuse every tool for it — the
/// failure mode is a session that cannot do anything, which is much worse than
/// a filter that briefly did not apply.
#[rstest::rstest]
#[tokio::test]
async fn a_session_absent_from_state_does_not_withhold_tools() {
    // Given an orchestrator whose state holds no session at all.
    let harness = TestHarness::new().await;
    spawn(&harness, State::new(jinn_kernel::AppState::default())).await;
    let session_id = SessionId::new();

    // When the model calls bash.
    let result = call_bash(&harness, &session_id).await;

    // Then the call runs rather than being refused by a filter that does not
    // exist. The two existing fabric tests dispatch against exactly such a
    // session; this pins the reason they still pass.
    assert!(
        result.success,
        "an absent session must not withhold; got: {:?}",
        result.content
    );
}
