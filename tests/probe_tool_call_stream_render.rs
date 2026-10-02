//! Gate: a streaming tool call's arguments must be visible as they arrive.
//!
//! The unit tests in `jinn-session-turn` hand-call `on_tool_use_started` and
//! `on_tool_call_streaming` on an actor the test itself already placed in
//! `Streaming`. That proves the append works *given* correct registration; it
//! does not prove registration is correct on the path production actually
//! takes. This module drives the real path instead: messages go onto the same
//! bus the actor was spawned on and are delivered by schema broadcast to a
//! real spawned session actor, with the phase in whatever state dispatch left
//! it in. Then the real `ChatLogElement` paints a frame and the buffer is
//! inspected.
//!
//! It lives in the root crate's `tests/` because it is the only place a test
//! may depend on the session-turn slice and the chat-log slice at once.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use std::time::Duration;

use jinn_inference_msg::StreamToken;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::render_ctx::RenderCtx;
use jinn_kernel::common::services::BusService;
use jinn_kernel::common::state::State;
use jinn_kernel::common::ui_element::UiElement;
use jinn_kernel::protocol::ChatEntryKind;
use jinn_session_turn::session_actor::SessionPersistenceActorDeps;
use jinn_tools_msg::{ToolCallReceived, ToolCallStreaming, ToolUseStarted};

/// A multi-line `write` payload, split the way a provider streams it.
///
/// The line breaks matter: this is the case the report is about, and the
/// renderer has to unescape them for the content to read as file content
/// rather than as a one-line JSON blob.
const WRITE_ARGS: &[&str] = &[
    "{\"file_path\":\"src/lib.rs\",\"content\":\"fn main() {\\n    println!(\\\"hello\\\");\\n}\\n\"",
    ",\"mode\":\"overwrite\"}",
];

/// Settles async bus delivery. A real publish hands the message to the
/// actor's mailbox; the state write lands on a later task turn.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(150)).await;
}

/// A live session actor over a real bus, plus the bus and the state handle
/// the chat log reads.
///
/// The bus is returned so the test publishes on the *same* bus the actor
/// subscribed to — a second `Services` would be a second fabric, and
/// nothing would be delivered at all.
async fn live_session() -> (BusService, State) {
    let services = jinn_kernel::Services::new_fake().await;
    let app = AppState::default_with_scope_focus();
    app.frontend.scope_clear_overlays();
    let state = State::new(app);

    jinn_session_turn::activate(
        &services.trouper_system,
        SessionPersistenceActorDeps {
            deps: jinn_kernel::common::actor_deps::ActorDeps {
                services: services.clone(),
            },
            state: state.clone(),
            counter: jinn_llm_support::token_estimator::TiktokenCounter::o200k_base(),
            token_cache: jinn_token_count_msg::HistoryWorkerChatEntryTokenCache::default(),
            image_converter: jinn_llm_support::image_convert::ImageConverterService::unavailable(),
        },
    );
    settle().await;
    (services.bus.clone(), state)
}

/// The active session id of a state handle.
fn active_session_id(state: &State) -> jinn_core_types::SessionId {
    state.read().session.active_session_id().clone()
}

/// Paints one frame of the chat log and returns the buffer's text.
fn painted(state: &AppState, width: u16, height: u16) -> String {
    let (mut terminal, area) = jinn_testutil::setup_term(width, height);
    let mut element = jinn_chat_log_view::kernel_element::ChatLogElement::new();
    let guard = state;
    terminal
        .draw(|frame| {
            let slices = jinn_slices::Slices::new();
            let overlay_views = jinn_slices::OverlayViews::new();
            let ctx = RenderCtx::new_with_default_config(guard, &slices, &overlay_views);
            element.render(frame, area, &ctx);
        })
        .expect("draw one chat log frame");
    let buffer = terminal.backend().buffer().clone();
    jinn_testutil::buffer_rows(&buffer, width, height).join("\n")
}

#[rstest::rstest]
#[tokio::test]
async fn streamed_tool_call_arguments_are_visible_before_the_call_completes() {
    // Given a live session actor and a turn dispatched exactly as the
    // user-message path dispatches it: `Sending`, no phase seeding here.
    let (bus, state) = live_session().await;
    let session_id = active_session_id(&state);
    state.write().active_session_mut().begin_sending();

    // When a provider begins the turn: a leading assistant token, then a
    // `write` tool call whose arguments stream in.
    bus.publish(StreamToken {
        session_id: session_id.clone(),
        index: 0,
        token: "Let me write that file.".to_owned(),
        is_thinking: false,
        dispatched_at: jiff::Timestamp::now(),
    })
    .await;
    settle().await;
    bus.publish(ToolUseStarted {
        session_id: session_id.clone(),
        index: 0,
        id: "tc-write".to_owned(),
        name: "write".to_owned(),
        dispatched_at: jiff::Timestamp::now(),
    })
    .await;
    settle().await;
    bus.publish(ToolCallStreaming {
        session_id: session_id.clone(),
        index: 0,
        partial_json: WRITE_ARGS[0].to_owned(),
    })
    .await;
    settle().await;
    let frame = painted(&state.read(), 80, 20);

    // Then the streamed bytes are on screen, not merely in session state.
    assert!(
        frame.contains("println!"),
        "expected the streamed argument bytes to be painted; got:\n{frame}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn streamed_arguments_are_visible_when_the_turn_starts_with_a_tool_call() {
    // Given a live session actor and a turn dispatched as the user-message
    // path dispatches it: `Sending`, and still `Sending` because no text
    // token has arrived — the shape of a model that opens by calling a tool.
    let (bus, state) = live_session().await;
    let session_id = active_session_id(&state);
    state.write().active_session_mut().begin_sending();

    // When the provider goes straight to the tool call and streams arguments.
    bus.publish(ToolUseStarted {
        session_id: session_id.clone(),
        index: 0,
        id: "tc-write".to_owned(),
        name: "write".to_owned(),
        dispatched_at: jiff::Timestamp::now(),
    })
    .await;
    settle().await;
    bus.publish(ToolCallStreaming {
        session_id: session_id.clone(),
        index: 0,
        partial_json: WRITE_ARGS[0].to_owned(),
    })
    .await;
    settle().await;
    let frame = painted(&state.read(), 80, 20);

    // Then the streamed bytes are on screen.
    assert!(
        frame.contains("println!"),
        "expected the streamed argument bytes to be painted; got:\n{frame}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn a_streamed_delta_lands_on_the_tool_call_entry_it_started_on() {
    // Given a live session actor mid-turn with a begun tool call.
    let (bus, state) = live_session().await;
    let session_id = active_session_id(&state);
    state.write().active_session_mut().begin_sending();
    bus.publish(StreamToken {
        session_id: session_id.clone(),
        index: 0,
        token: "Let me write that file.".to_owned(),
        is_thinking: false,
        dispatched_at: jiff::Timestamp::now(),
    })
    .await;
    settle().await;
    bus.publish(ToolUseStarted {
        session_id: session_id.clone(),
        index: 0,
        id: "tc-write".to_owned(),
        name: "write".to_owned(),
        dispatched_at: jiff::Timestamp::now(),
    })
    .await;
    bus.publish(ToolCallStreaming {
        session_id: session_id.clone(),
        index: 0,
        partial_json: WRITE_ARGS[0].to_owned(),
    })
    .await;
    settle().await;

    // When the call completes and the finalized arguments arrive.
    bus.publish(ToolCallReceived {
        session_id: session_id.clone(),
        tool_call: jinn_core_types::tool_types::ToolCall {
            id: "tc-write".to_owned(),
            name: "write".to_owned(),
            arguments: format!("{}{}", WRITE_ARGS[0], WRITE_ARGS[1]),
        },
        dispatched_at: jiff::Timestamp::now(),
    })
    .await;
    settle().await;

    // Then the entry carries the payload, proving the deltas landed on the
    // entry the call started on rather than being dropped.
    let guard = state.read();
    let session = guard.session.get(&session_id).expect("session");
    let call = session
        .history()
        .iter()
        .find(|e| matches!(&e.kind, ChatEntryKind::ToolCall { id, .. } if id == "tc-write"))
        .expect("tool call entry");
    let ChatEntryKind::ToolCall { arguments, .. } = &call.kind else {
        panic!("expected a ToolCall entry");
    };
    assert!(
        arguments.contains("println!"),
        "expected arguments to hold the streamed payload, got: {arguments}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_streamed_display_grows_monotonically_as_deltas_arrive() {
    // Given a live session actor mid-turn with one started tool call.
    let (bus, state) = live_session().await;
    let session_id = active_session_id(&state);
    state.write().active_session_mut().begin_sending();
    bus.publish(ToolUseStarted {
        session_id: session_id.clone(),
        index: 0,
        id: "tc-write".to_owned(),
        name: "write".to_owned(),
        dispatched_at: jiff::Timestamp::now(),
    })
    .await;
    settle().await;

    // When three deltas arrive and the log paints after each.
    let mut frames = Vec::new();
    for delta in WRITE_ARGS.iter().chain(std::iter::once(&"")).take(2) {
        bus.publish(ToolCallStreaming {
            session_id: session_id.clone(),
            index: 0,
            partial_json: (*delta).to_owned(),
        })
        .await;
        settle().await;
        frames.push(painted(&state.read(), 80, 20));
    }

    // Then each frame shows strictly more of the payload than the last.
    let first = frames[0].find("println!").expect("payload in first frame");
    let second = frames[1].find("println!").expect("payload in second frame");
    assert!(
        second >= first,
        "the streamed display did not grow: {first} then {second}"
    );
    assert!(
        frames[1].contains("overwrite"),
        "expected the second delta's bytes on screen; got:\n{}",
        frames[1]
    );
}

#[rstest::rstest]
#[case("write")]
#[case("edit")]
#[case("read")]
#[case("bash")]
#[tokio::test]
async fn every_streaming_tool_paints_its_arguments(#[case] tool: &str) {
    // Given a live session actor mid-turn.
    let (bus, state) = live_session().await;
    let session_id = active_session_id(&state);
    state.write().active_session_mut().begin_sending();

    // When a tool call of this shape streams its arguments.
    bus.publish(ToolUseStarted {
        session_id: session_id.clone(),
        index: 0,
        id: "tc-1".to_owned(),
        name: tool.to_owned(),
        dispatched_at: jiff::Timestamp::now(),
    })
    .await;
    settle().await;
    bus.publish(ToolCallStreaming {
        session_id: session_id.clone(),
        index: 0,
        partial_json: WRITE_ARGS[0].to_owned(),
    })
    .await;
    settle().await;
    let frame = painted(&state.read(), 80, 20);

    // Then the streamed bytes are painted, with no per-tool special case.
    assert!(
        frame.contains("println!"),
        "expected `{tool}` arguments to be painted; got:\n{frame}"
    );
}
