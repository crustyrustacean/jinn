//! Tests for the `message_session` tool — the four delivery paths and the
//! no-silent-drop invariant.

#![allow(clippy::expect_used, reason = "test code")]

use std::time::Duration;

use jinn_chat_input_msg::EnqueueUserMessage;
use jinn_common::app_paths::AppPaths;
use jinn_core_types::SessionId;
use jinn_core_types::chat_entry::ChatEntry;
use jinn_core_types::tool_types::ToolCall;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::state::State;
use jinn_session_state::ChatSessionState;
use jinn_session_store_msg::PersistSession;
use jinn_testutil::bus_harness::{TestHarness, await_recorded};

use crate::message_session::{definition, execute};
use crate::tool_types::ToolContext;

/// Builds a tool call with the given JSON arguments.
fn call(arguments: &str) -> ToolCall {
    ToolCall {
        id: "tc_msg_1".to_owned(),
        name: "message_session".to_owned(),
        arguments: arguments.to_owned(),
    }
}

/// Builds a tool context wired to the harness bus and shared state.
async fn ctx(harness: &TestHarness, state: &State, session_id: SessionId) -> ToolContext {
    ToolContext {
        cwd: std::path::PathBuf::from("/tmp"),
        config: jinn_config::testutil::config_layer(""),
        timeout: None,
        state: Some(state.clone()),
        session_id: Some(session_id),
        app_paths: AppPaths::new_in(std::path::Path::new("/tmp")),
        bus: Some(harness.bus()),
        max_output_lines: None,
        max_output_bytes: None,
        dispatched_at: jiff::Timestamp::now(),
        mcp_coordinator: None,
        interactive_term: None,
        task_spawns: None,
        session_store: None,
        trouper_system: Some(harness.system().clone()),
        model_name_resolver: None,
    }
}

/// Seeds a user session and an attendant with a parent and a seed template.
fn fixture() -> (State, SessionId, SessionId) {
    let state = State::new(AppState::default_with_scope_focus());
    let user = ChatSessionState::new();
    let user_id = user.session_id().clone();
    let attendant = ChatSessionState::new_attendant(&user, true);
    let attendant_id = attendant.session_id().clone();
    {
        let mut guard = state.write();
        guard.session.insert(user);
        guard.session.insert(attendant);
    }
    (state, user_id, attendant_id)
}

/// Sets the attendant's seed template so seeded runs are recognizable.
fn set_template(state: &State, attendant_id: &SessionId, template: &str) {
    state
        .write()
        .session
        .get_mut(attendant_id)
        .expect("attendant")
        .set_seed_template(template.to_owned());
}

/// Ends prep mode so the attendant can run.
fn take_out_of_prep(state: &State, attendant_id: &SessionId) {
    state
        .write()
        .session
        .get_mut(attendant_id)
        .expect("attendant")
        .set_attendant_is_prepping(false);
}

/// Drives the session into the Streaming phase, like a real turn.
fn make_busy(state: &State, session_id: &SessionId) {
    state
        .write()
        .session
        .get_mut(session_id)
        .expect("session")
        .begin_streaming();
}

#[rstest::rstest]
#[tokio::test]
async fn idle_attendant_starts_a_run_seeded_with_the_message() {
    // Given an idle attendant with a seed template.
    let harness = TestHarness::new().await;
    let enqueues = harness.spawn_recorder::<EnqueueUserMessage>().await;
    let (state, user_id, attendant_id) = fixture();
    take_out_of_prep(&state, &attendant_id);
    set_template(&state, &attendant_id, "Review the diff. <prior report>");
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;

    // When a message is sent to the attendant.
    let result = execute(
        call(&format!(
            r#"{{"session_id":"{attendant_id}","message":"focus on main.rs"}}"#
        )),
        tool_ctx,
    )
    .await;

    // Then the tool succeeds and one seeded enqueue is published.
    assert!(result.success, "delivery must succeed: {}", result.content);
    let recorded = await_recorded::<EnqueueUserMessage>(&enqueues, 1, Duration::from_secs(2)).await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].session_id, attendant_id);
    // And the dispatched text carries the template seed and the message.
    let jinn_core_types::chat_entry::ChatEntryKind::User { expanded, .. } = &recorded[0].entry.kind
    else {
        panic!("seeded run must dispatch as a user entry");
    };
    assert!(expanded.contains("Review the diff."), "seed present");
    assert!(expanded.contains("focus on main.rs"), "message present");
    // And the target was marked interacted so its run persists.
    let guard = state.read();
    assert!(
        guard
            .session
            .get(&attendant_id)
            .expect("attendant")
            .has_interacted()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn idle_attendant_with_empty_template_dispatches_the_message_alone() {
    // Given an idle attendant whose template renders to nothing.
    let harness = TestHarness::new().await;
    let enqueues = harness.spawn_recorder::<EnqueueUserMessage>().await;
    let (state, user_id, attendant_id) = fixture();
    take_out_of_prep(&state, &attendant_id);
    set_template(&state, &attendant_id, "");
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;

    // When a message is sent.
    let result = execute(
        call(&format!(
            r#"{{"session_id":"{attendant_id}","message":"just do the thing"}}"#
        )),
        tool_ctx,
    )
    .await;

    // Then the dispatched entry is the message alone.
    assert!(result.success, "{}", result.content);
    let recorded = await_recorded::<EnqueueUserMessage>(&enqueues, 1, Duration::from_secs(2)).await;
    let jinn_core_types::chat_entry::ChatEntryKind::User { expanded, .. } = &recorded[0].entry.kind
    else {
        panic!("seeded run must dispatch as a user entry");
    };
    assert_eq!(expanded, "just do the thing");
}

#[rstest::rstest]
#[tokio::test]
async fn busy_attendant_queues_and_never_cancels() {
    // Given a busy attendant (mid-stream) with a recorder for cancel commands.
    let harness = TestHarness::new().await;
    let enqueues = harness.spawn_recorder::<EnqueueUserMessage>().await;
    let cancels = harness
        .spawn_recorder::<jinn_inference_msg::CancelTurn>()
        .await;
    let (state, user_id, attendant_id) = fixture();
    take_out_of_prep(&state, &attendant_id);
    set_template(&state, &attendant_id, "Review the diff.");
    make_busy(&state, &attendant_id);
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;

    // When a message is sent to the busy attendant.
    let result = execute(
        call(&format!(
            r#"{{"session_id":"{attendant_id}","message":"also check tests"}}"#
        )),
        tool_ctx,
    )
    .await;

    // Then the seeded enqueue is published and no cancel is.
    assert!(result.success, "{}", result.content);
    assert!(
        result.content.contains("queued behind its current turn"),
        "busy delivery must say so: {}",
        result.content
    );
    let recorded = await_recorded::<EnqueueUserMessage>(&enqueues, 1, Duration::from_secs(2)).await;
    assert_eq!(recorded[0].session_id, attendant_id);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        cancels.is_empty(),
        "delivery must never cancel in-flight work"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn plain_session_receives_the_message_as_a_user_turn() {
    // Given a non-attendant session with no title yet.
    let harness = TestHarness::new().await;
    let enqueues = harness.spawn_recorder::<EnqueueUserMessage>().await;
    let (state, user_id, _attendant_id) = fixture();
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;

    // When a message is sent to it.
    let result = execute(
        call(&format!(
            r#"{{"session_id":"{user_id}","message":"orchestrator says hello"}}"#
        )),
        tool_ctx,
    )
    .await;

    // Then the enqueue carries a plain User entry addressed to that session.
    assert!(result.success, "{}", result.content);
    let recorded = await_recorded::<EnqueueUserMessage>(&enqueues, 1, Duration::from_secs(2)).await;
    assert_eq!(recorded[0].session_id, user_id);
    assert!(
        matches!(
            &recorded[0].entry.kind,
            jinn_core_types::chat_entry::ChatEntryKind::User { .. }
        ),
        "plain delivery must be a User entry or it mints an empty title"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn unknown_session_id_fails_naming_the_target() {
    // Given a state with no session bearing the target id.
    let harness = TestHarness::new().await;
    let enqueues = harness.spawn_recorder::<EnqueueUserMessage>().await;
    let (state, user_id, _attendant_id) = fixture();
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;
    let missing = SessionId::new();

    // When a message is sent to the unknown id.
    let result = execute(
        call(&format!(
            r#"{{"session_id":"{missing}","message":"anyone there?"}}"#
        )),
        tool_ctx,
    )
    .await;

    // Then the tool fails naming the target and publishes nothing.
    assert!(!result.success, "unknown target must fail, not vanish");
    assert!(result.content.contains(&missing.to_string()));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(enqueues.is_empty(), "a refused send must publish nothing");
}

#[rstest::rstest]
#[tokio::test]
async fn malformed_session_id_fails_without_publishing() {
    // Given a call whose session_id is not a UUID.
    let harness = TestHarness::new().await;
    let enqueues = harness.spawn_recorder::<EnqueueUserMessage>().await;
    let (state, user_id, _attendant_id) = fixture();
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;

    // When the tool runs with the malformed id.
    let result = execute(
        call(r#"{"session_id":"not-a-uuid","message":"hi"}"#),
        tool_ctx,
    )
    .await;

    // Then it fails naming the argument and publishes nothing.
    assert!(!result.success);
    assert!(result.content.contains("not-a-uuid"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(enqueues.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn prep_mode_attendant_refuses_delivery() {
    // Given an attendant still being composed (prep mode is the default).
    let harness = TestHarness::new().await;
    let enqueues = harness.spawn_recorder::<EnqueueUserMessage>().await;
    let (state, user_id, attendant_id) = fixture();
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;
    assert!(
        state
            .read()
            .session
            .get(&attendant_id)
            .expect("attendant")
            .attendant_is_prepping(),
        "fixture must start in prep mode"
    );

    // When a message is sent to the prepping attendant.
    let result = execute(
        call(&format!(
            r#"{{"session_id":"{attendant_id}","message":"wake up"}}"#
        )),
        tool_ctx,
    )
    .await;

    // Then delivery is refused with the reason and nothing is published.
    assert!(!result.success);
    assert!(result.content.contains("prep mode"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(enqueues.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn reset_behavior_attendant_persists_the_exclusions() {
    // Given an idle attendant in the default Reset behavior with history.
    let harness = TestHarness::new().await;
    let enqueues = harness.spawn_recorder::<EnqueueUserMessage>().await;
    let persists = harness.spawn_recorder::<PersistSession>().await;
    let (state, user_id, attendant_id) = fixture();
    {
        let mut guard = state.write();
        let attendant = guard.session.get_mut(&attendant_id).expect("attendant");
        attendant.set_attendant_is_prepping(false);
        attendant.push_entry(ChatEntry::user("old context"));
    }
    let tool_ctx = ctx(&harness, &state, user_id.clone()).await;

    // When a message is delivered.
    let result = execute(
        call(&format!(
            r#"{{"session_id":"{attendant_id}","message":"again"}}"#
        )),
        tool_ctx,
    )
    .await;

    // Then a PersistSession for the attendant rides ahead of the enqueue.
    assert!(result.success, "{}", result.content);
    let persisted = await_recorded::<PersistSession>(&persists, 1, Duration::from_secs(2)).await;
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].session_id, attendant_id);
    // And the non-pinned history is force-excluded.
    let guard = state.read();
    let attendant = guard.session.get(&attendant_id).expect("attendant");
    assert!(
        attendant.history().iter().all(|e| e.is_pinned()
            || e.context_override == jinn_core_types::ContextOverride::ForcedExclude)
    );
    let _ = enqueues; // the enqueue itself is covered by the idle test
}

#[rstest::rstest]
#[test]
fn definition_requires_both_arguments_and_names_the_tool() {
    // Given the tool definition.
    let def = definition();

    // Then it is named message_session with both parameters required.
    assert_eq!(def.name, "message_session");
    let required = def.parameters["required"].as_array().expect("required");
    assert_eq!(required.len(), 2);
}
