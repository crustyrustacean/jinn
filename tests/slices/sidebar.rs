//! End-to-end crossing test for the sidebar slice: the session store publishes
//! `SessionRemoved` on the kernel topic → route rule → `jinn.sidebar`
//! topic → the slice's trouper state actor → sidebar cursor clamped.
//!
//! The cursor is observable through the sidebar sections cell — the same
//! view the renderer reads — so only a true crossing can satisfy the
//! assertion.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use std::sync::Arc;
use std::time::Duration;

use jinn_chat_log_view_msg::PreviewSessionRequested;
use jinn_core_types::ChatEntry;
use jinn_domain::common::bridge::Bridge;
use jinn_session_msg::SessionRemoved;
use jinn_session_state::ChatSessionState;

use crate::common::test_app;

/// Polls `read` until it returns `Some` (up to `timeout`), else panics.
async fn await_condition<R>(timeout: Duration, mut read: impl FnMut() -> Option<R>) -> R
where
    R: std::fmt::Debug + PartialEq + Send + 'static,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(value) = read() {
            return value;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("condition not observed within {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The sidebar cursor is clamped after the kernel closes a session:
/// bus → relay → trouper actor → sections cell.
#[rstest::rstest]
#[tokio::test]
async fn session_closed_crosses_to_sidebar_and_clamps_cursor() {
    // Given a composed app whose sidebar cursor sits at index 2 of
    // three sessions, with the third session already removed (the
    // close itself — the actor's job is the cursor clamp).
    let app = test_app().await;
    let removed_id = {
        let mut state = app.core.state.write();
        let default_id = state.session.active_session_id().clone();
        state.session.remove_without_replacement(&default_id);
        let s1 = ChatSessionState::new();
        let s2 = ChatSessionState::new();
        let s3 = ChatSessionState::new();
        let id3 = s3.session_id().clone();
        state.session.insert(s1);
        state.session.insert(s2);
        state.session.insert(s3);
        state.session.set_active(id3.clone());
        state
            .frontend
            .update_sections(|s| s.sessions.selected_index = Some(2));
        state.session.remove_without_replacement(&id3);
        id3
    };

    // When the session store publishes `SessionRemoved` on the bus.
    let _ = app
        .core
        .bridge
        .send(Bridge::publish_closure(SessionRemoved {
            session_id: removed_id,
            removed_parent: None,
        }));

    // Then the sidebar cursor is clamped to 1 (max valid index).
    let clamped = await_condition(Duration::from_secs(5), || {
        app.core
            .state
            .read()
            .frontend
            .with_sections(|s| s.sessions.selected_index, || None)
            .filter(|index| *index == 1)
    })
    .await;
    assert_eq!(clamped, 1);
}

/// The preview deadline actually reaches the layout supervisor.
///
/// The sidebar state actor arms a deadline for every preview it handles, and the
/// supervisor owns the timer that stops a stuck render from spinning forever.
/// That crossing is invisible to every unit test: the actor's handler is
/// exercised directly, and the supervisor's handler likewise, so both pass while
/// the message between them is dropped by the bus's flush gate — the actor never
/// declaring it emits. The observable consequence is a preview that spins
/// forever once a render is genuinely lost.
#[rstest::rstest]
#[tokio::test]
// The preview deadline is 5s of real time, so the test's own wait has to
// outlast it. The `#[timeout]` above rstest's default of 10s must be raised to
// match, or the harness kills the test mid-wait.
#[timeout(Duration::from_secs(20))]
async fn a_preview_request_reaches_the_layout_supervisor_as_a_deadline() {
    // Given a composed app with a session whose preview is being requested.
    let app = test_app().await;
    let session_id = {
        let state = app.core.state.read();
        state.session.active_session_id().clone()
    };

    // When the preview request crosses the bus.
    let generation = {
        let state = app.core.state.read();
        state
            .frontend
            .update_sections(|s| s.sessions.preview.request(session_id.clone(), 7, 103))
            .expect("the sections cell is attached")
    };
    let _ = app
        .core
        .bridge
        .send(Bridge::publish_closure(PreviewSessionRequested {
            session_id: session_id.clone(),
            content_width: 103,
            generation,
            entries: Arc::from(Vec::<ChatEntry>::new()),
            tool_entry_max_lines: 6,
            signature: 7,
        }));

    // Then the request is abandoned on its deadline, which can only happen if
    // the actor emitted `ArmPreviewDeadline` and the supervisor handled it.
    let abandoned = await_condition(Duration::from_secs(20), || {
        let state = app.core.state.read();
        (!state.frontend.with_sections(
            |s| s.sessions.preview.is_in_flight_for(&session_id),
            || false,
        ))
        .then_some(true)
    })
    .await;
    assert!(
        abandoned,
        "the preview deadline must fire and abandon the in-flight request (in_flight at start: {})",
        app.core.state.read().frontend.with_sections(
            |s| s.sessions.preview.is_in_flight_for(&session_id),
            || false,
        ),
    );
}
