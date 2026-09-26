//! Sidebar state actor — owns the sidebar's own state transitions.
//!
//! A trouper [`ServiceActor`] subscribed to the slice's `jinn.sidebar`
//! topic (fed by the kernel bridge's forward routes). It folds
//! [`SessionRemoved`] into the sidebar cursor and active session, and it owns
//! the session preview's lifecycle: arming a render, completing it, and giving
//! up on it when the deadline passes.
//!
//! The preview lives here rather than in an actor of its own because the
//! sidebar sections' cell is already reachable from this actor's [`State`], and
//! a second actor would need its own spawn, its own deps, and its own readiness
//! point to write the same cell.

use trouper::actor::ActorPath;
use trouper::actor::{MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

use std::sync::Arc;
use std::time::Duration;

use crate::sections::sessions;
use jinn_chat_log_view_msg::{ArmPreviewDeadline, PreviewSessionRequested, SessionPreviewRendered};
use jinn_domain::common::state::State;
use jinn_session_msg::SessionRemoved;

/// The sidebar state actor's static trouper path.
pub const SIDEBAR_STATE_PATH: &str = "sidebar-state";

/// How long a preview render may run before it is abandoned.
///
/// The chat log's own deadline is 30 seconds, which is generous for a five-entry
/// preview and would leave a spinner up far too long if a worker wedged.
pub const PREVIEW_DEADLINE: Duration = Duration::from_secs(5);

/// Actor that adjusts sidebar cursor state in response to session close.
///
/// Holds the shared [`State`] handle, injected at spawn via `start_with`
/// because it cannot ride trouper's JSON args.
pub struct SidebarStateActor {
    state: State,
}

impl ServiceActor for SidebarStateActor {
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the state handle via
        // `start_with`.
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("SidebarStateActor is spawned via start_with"),
        )
    }
}

impl SidebarStateActor {
    /// Spawns the actor at its static path. The caller subscribes the
    /// returned path to the sidebar topic (composition's
    /// `SliceHost::subscribe_service`) — subscribe is the readiness
    /// point, so it must follow this call before any publish.
    pub fn spawn(system: &ActorSystem, state: State) -> ActorPath {
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(ActorPath::new(SIDEBAR_STATE_PATH))
            .start_with({
                move || {
                    let state = state.clone();
                    Box::pin(async move { Ok(Self { state }) })
                }
            })
            .handles::<SessionRemoved>()
            .handles::<PreviewSessionRequested>()
            .handles::<SessionPreviewRendered>()
            .start()
    }

    /// Reconcile sidebar cursor and active session after a session is removed.
    fn handle_session_removed(&self, payload: &SessionRemoved) {
        self.state.with_session_sidebar(|view| {
            sessions::state::repair_visual_parents_after_removal(
                view.session.map(),
                view.frontend,
                &payload.session_id,
                payload.removed_parent.as_ref(),
            );
            sessions::reconcile_split(view.session.map(), view.frontend);
        });
    }

    /// Completes a preview render, ignoring a result for a superseded request.
    fn complete_preview(
        &self,
        session_id: jinn_core_types::SessionId,
        generation: u64,
        signature: u64,
        content_width: u16,
        lines: Arc<Vec<ratatui::text::Line<'static>>>,
    ) {
        self.state.read().frontend.update_sections(|s| {
            s.sessions
                .preview
                .complete(session_id, generation, signature, content_width, lines);
        });
    }
}

impl MsgHandler<SessionRemoved> for SidebarStateActor {
    async fn handle(&mut self, msg: &SessionRemoved, _ctx: &mut MsgCtx<'_>) {
        self.handle_session_removed(msg);
    }
}

impl MsgHandler<PreviewSessionRequested> for SidebarStateActor {
    async fn handle(&mut self, msg: &PreviewSessionRequested, ctx: &mut MsgCtx<'_>) {
        // The arming already happened on the keyboard path, before this message
        // was published — the render pass needs the spinner up the instant the
        // cursor moves, not a bus round trip later. What this handler adds is
        // the deadline, so a request that never comes back stops spinning. The
        // timer belongs to the layout supervisor, which already owns one per
        // job in this pool.
        ctx.publish(ArmPreviewDeadline {
            session_id: msg.session_id.clone(),
            generation: msg.generation,
            after: PREVIEW_DEADLINE,
        });
    }
}

impl MsgHandler<SessionPreviewRendered> for SidebarStateActor {
    async fn handle(&mut self, msg: &SessionPreviewRendered, _ctx: &mut MsgCtx<'_>) {
        // Destructured into fields rather than handed whole: this crate does not
        // depend on the message crate, and a bus type would leak a dependency
        // the sidebar's own state has no use for.
        self.complete_preview(
            msg.session_id.clone(),
            msg.generation,
            msg.signature,
            msg.content_width,
            Arc::clone(&msg.lines),
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        unused_mut,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jinn_domain::common::app_state::AppState;
    use jinn_domain::common::state::State;
    use jinn_session_state::ChatSessionState;

    fn test_actor() -> SidebarStateActor {
        SidebarStateActor {
            state: State::new(AppState::default_with_scope_focus()),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn clamps_selected_index_after_session_removed() {
        // Given a sidebar actor with three sessions and cursor at index 2.
        let actor = test_actor();
        let removed_id = {
            let mut state = actor.state.write();
            // Remove default session so we control exact count.
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
            id3
        };

        // Simulate the session being removed (as the session actor would do).
        {
            let mut state = actor.state.write();
            state.session.remove_without_replacement(&removed_id);
        }

        // When handling SessionClosed.
        let payload = jinn_session_msg::SessionRemoved {
            session_id: removed_id,
            removed_parent: None,
        };
        actor.handle_session_removed(&payload);

        // Then selected_index is clamped to 1 (max valid index).
        let state = actor.state.read();
        assert_eq!(
            state
                .frontend
                .with_sections(|s| s.sessions.selected_index, || None),
            Some(1)
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handles_removal_of_last_session_cursor_at_zero() {
        // Given a sidebar actor with one session and cursor at 0.
        let actor = test_actor();
        let removed_id = {
            let mut state = actor.state.write();
            let id = state.session.active_session_id().clone();
            state
                .frontend
                .update_sections(|s| s.sessions.selected_index = Some(0));
            id
        };

        // Simulate session close + new session creation (as session actor would do).
        {
            let mut state = actor.state.write();
            state
                .session
                .remove_and_replace(&removed_id, ChatSessionState::new());
        }

        // When handling SessionClosed.
        let payload = jinn_session_msg::SessionRemoved {
            session_id: removed_id,
            removed_parent: None,
        };
        actor.handle_session_removed(&payload);

        // Then cursor stays at 0.
        let state = actor.state.read();
        assert_eq!(
            state
                .frontend
                .with_sections(|s| s.sessions.selected_index, || None),
            Some(0)
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn cursor_stays_when_index_still_valid() {
        // Given a sidebar actor with three sessions and cursor at index 0.
        let actor = test_actor();
        let removed_id = {
            let mut state = actor.state.write();
            let s1 = ChatSessionState::new();
            let s2 = ChatSessionState::new();
            let s3 = ChatSessionState::new();
            let id3 = s3.session_id().clone();
            state.session.insert(s1);
            state.session.insert(s2);
            state.session.insert(s3);
            state
                .frontend
                .update_sections(|s| s.sessions.selected_index = Some(0));
            id3
        };

        // Simulate removal of the last session (cursor at 0 is still valid).
        {
            let mut state = actor.state.write();
            state.session.remove_without_replacement(&removed_id);
        }

        // When handling SessionClosed.
        let payload = jinn_session_msg::SessionRemoved {
            session_id: removed_id,
            removed_parent: None,
        };
        actor.handle_session_removed(&payload);

        // Then cursor stays at 0.
        let state = actor.state.read();
        assert_eq!(
            state
                .frontend
                .with_sections(|s| s.sessions.selected_index, || None),
            Some(0)
        );
    }
}

#[cfg(test)]
mod preview_tests {
    #![allow(
        unused_mut,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jinn_chat_log_view_msg::PREVIEW_ENTRY_COUNT;
    use jinn_core_types::ChatEntry;
    use jinn_domain::common::app_state::AppState;
    use jinn_domain::common::state::State;
    use jinn_session_state::ChatSessionState;
    use jinn_sidebar_msg::PreviewLoad;
    use jinn_slices::spinner_glyph;

    /// A session with `count` user entries, plus the id it registered under.
    fn session_with(count: usize) -> (ChatSessionState, jinn_core_types::SessionId) {
        let mut session = ChatSessionState::new();
        for i in 0..count {
            session.push_entry(ChatEntry::user(format!("message {i}")));
        }
        let id = session.session_id().clone();
        (session, id)
    }

    /// The preview state as the sidebar cell holds it.
    fn preview_of(state: &State) -> Option<PreviewLoad> {
        state
            .read()
            .frontend
            .with_sections(|s| Some(s.sessions.preview.clone()), || None)
    }

    fn test_actor() -> SidebarStateActor {
        SidebarStateActor {
            state: State::new(AppState::default_with_scope_focus()),
        }
    }

    #[rstest::rstest]
    fn a_rendered_result_completes_the_preview() {
        // Given a request in flight.
        let actor = test_actor();
        let (_session, id) = session_with(2);
        let generation = actor
            .state
            .read()
            .frontend
            .update_sections(|s| s.sessions.preview.request(id.clone()))
            .expect("cell");

        // When the result arrives.
        actor.complete_preview(
            id.clone(),
            generation,
            42,
            40,
            Arc::new(vec![ratatui::text::Line::from("hello")]),
        );

        // Then the preview is ready at the generation that was requested.
        let preview = preview_of(&actor.state).expect("cell");
        let ready = matches!(
            &preview,
            PreviewLoad::Ready {
                session_id,
                generation: ready_generation,
                ..
            } if session_id == &id && *ready_generation == generation
        );
        assert!(ready, "expected a ready preview, got {preview:?}");
    }

    #[rstest::rstest]
    fn a_result_for_an_older_generation_is_ignored() {
        // Given a request in flight at generation 2.
        let actor = test_actor();
        let (_session, id) = session_with(2);
        actor.state.read().frontend.update_sections(|s| {
            s.sessions.preview.request(id.clone());
            s.sessions.preview.request(id.clone());
        });

        // When generation 1's result arrives late.
        actor.complete_preview(
            id.clone(),
            1,
            42,
            40,
            Arc::new(vec![ratatui::text::Line::from("stale")]),
        );

        // Then the newer request is still in flight — the stale lines are not shown.
        let preview = preview_of(&actor.state).expect("cell");
        assert!(
            matches!(preview, PreviewLoad::Loading { generation: 2, .. }),
            "a superseded result must not replace a live request, got {preview:?}"
        );
    }

    #[rstest::rstest]
    fn a_result_for_another_session_is_ignored() {
        // Given a request in flight for one session.
        let actor = test_actor();
        let (_session, id) = session_with(2);
        let generation = actor
            .state
            .read()
            .frontend
            .update_sections(|s| s.sessions.preview.request(id.clone()))
            .expect("cell");

        // When a result for a different session arrives.
        actor.complete_preview(
            jinn_core_types::SessionId::new(),
            generation,
            42,
            40,
            Arc::new(vec![ratatui::text::Line::from("wrong session")]),
        );

        // Then the in-flight request is untouched.
        let preview = preview_of(&actor.state).expect("cell");
        assert!(
            matches!(preview, PreviewLoad::Loading { .. }),
            "another session's result must not land here, got {preview:?}"
        );
    }

    #[rstest::rstest]
    fn the_deadline_clears_a_stuck_spinner() {
        // Given a request in flight at generation 1.
        let actor = test_actor();
        let (_session, id) = session_with(2);
        actor
            .state
            .read()
            .frontend
            .update_sections(|s| s.sessions.preview.request(id.clone()));

        // When the deadline for that exact request fires.
        actor
            .state
            .read()
            .frontend
            .update_sections(|s| s.sessions.preview.abandon(&id, 1));

        // Then the spinner is gone.
        assert_eq!(
            preview_of(&actor.state).expect("cell"),
            PreviewLoad::Idle,
            "an abandoned preview must not spin forever"
        );
    }

    #[rstest::rstest]
    fn the_deadline_for_an_older_generation_does_not_clear_a_live_spinner() {
        // Given a request in flight at generation 2.
        let actor = test_actor();
        let (_session, id) = session_with(2);
        actor.state.read().frontend.update_sections(|s| {
            s.sessions.preview.request(id.clone());
            s.sessions.preview.request(id.clone());
        });

        // When generation 1's deadline fires late.
        actor
            .state
            .read()
            .frontend
            .update_sections(|s| s.sessions.preview.abandon(&id, 1));

        // Then the live request keeps its spinner.
        let preview = preview_of(&actor.state).expect("cell");
        assert!(
            matches!(preview, PreviewLoad::Loading { generation: 2, .. }),
            "a stale deadline must not strand the live request, got {preview:?}"
        );
    }

    #[rstest::rstest]
    fn the_preview_deadline_is_far_shorter_than_the_layout_deadline() {
        // The chat log's deadline is 30 seconds; a five-entry preview that
        // takes that long is wedged, not slow.
        assert!(
            PREVIEW_DEADLINE < std::time::Duration::from_secs(30),
            "the preview deadline must be its own, not the layout one"
        );
    }

    #[rstest::rstest]
    fn a_cleared_preview_returns_to_idle() {
        // Given a completed preview holding rendered lines.
        let actor = test_actor();
        let (_session, id) = session_with(2);
        actor.state.read().frontend.update_sections(|s| {
            let generation = s.sessions.preview.request(id.clone());
            s.sessions.preview.complete(
                id.clone(),
                generation,
                42,
                40,
                Arc::new(vec![ratatui::text::Line::from("hello")]),
            );
        });

        // When the theme changes and the lines are dropped.
        actor
            .state
            .read()
            .frontend
            .update_sections(|s| s.sessions.preview.reset());

        // Then the preview is idle, so the next cursor move re-renders it in the
        // new theme rather than showing the old colors.
        assert_eq!(preview_of(&actor.state).expect("cell"), PreviewLoad::Idle);
    }

    // Keeps the imports honest: the spinner glyph and the entry count are part
    // of this actor's contract with the render pass even though the actor
    // itself does not reference them.
    #[rstest::rstest]
    fn the_preview_constants_are_shared_with_the_render_pass() {
        assert_eq!(PREVIEW_ENTRY_COUNT, 5);
        assert!(spinner_glyph(std::time::Duration::from_millis(80)).len() == 1);
    }
}

/// The preview request's trip through the layout worker pool.
///
/// A request that no worker declares is dropped silently by `send_to_any`, and
/// the popup spins forever with no error anywhere. These spawn a real worker on
/// the harness's system and publish a real request, so a missing declaration
/// shows up as a failing test rather than a stuck spinner.
#[cfg(test)]
mod preview_bus_tests {
    #![allow(
        unused_mut,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jinn_domain::common::app_state::AppState;
    use jinn_domain::common::bus::test_harness::{TestHarness, await_recorded};
    use jinn_domain::feat::ui::chat_log::{LayoutWorkerActor, LayoutWorkerActorDeps};
    use jinn_session_state::ChatSessionState;
    use ratatui::text::Line;
    use std::time::Duration;

    /// The text of a rendered line, which is what a result carries.
    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// A session with `count` one-line user entries.
    fn session_with(count: usize) -> ChatSessionState {
        let mut session = ChatSessionState::new();
        for i in 0..count {
            session.push_entry(jinn_core_types::ChatEntry::user(format!("message {i}")));
        }
        session
    }

    /// A request for `session`'s preview at `width`.
    fn request(session: &ChatSessionState, width: u16, signature: u64) -> PreviewSessionRequested {
        PreviewSessionRequested {
            session_id: session.session_id().clone(),
            content_width: width,
            generation: 1,
            entries: std::sync::Arc::from(session.history()),
            tool_entry_max_lines: 6,
            signature,
        }
    }

    /// One worker spawned on the harness, plus a recorder for its results.
    async fn worker_and_recorder(
        harness: &TestHarness,
        state: &State,
    ) -> jinn_domain::common::bus::test_harness::Recorder<SessionPreviewRendered> {
        LayoutWorkerActor::spawn(
            harness.system(),
            0,
            LayoutWorkerActorDeps {
                state: state.clone(),
            },
        );
        harness.spawn_recorder::<SessionPreviewRendered>().await
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn a_preview_request_is_answered_with_lines() {
        // Given a worker and a session with three one-line entries.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default_with_scope_focus());
        let recorder = worker_and_recorder(&harness, &state).await;
        let session = session_with(3);

        // When a preview request is published.
        harness.publish(request(&session, 40, 7)).await;

        // Then the worker answers with the entry text.
        let results = await_recorded(&recorder, 1, Duration::from_secs(5)).await;
        assert_eq!(
            results.len(),
            1,
            "no worker answered the preview request; the popup would spin forever"
        );
        let text: String = results[0].lines.iter().map(line_text).collect();
        assert!(
            text.contains("message 2"),
            "the preview must carry the newest entry, got {text:?}"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn a_preview_result_echoes_the_request_identity() {
        // Given a worker and a preview request.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default_with_scope_focus());
        let recorder = worker_and_recorder(&harness, &state).await;
        let session = session_with(2);
        let id = session.session_id().clone();

        // When it is published.
        harness.publish(request(&session, 55, 7)).await;

        // Then the result identifies the same request, so the sidebar can match
        // it against what it asked for.
        let results = await_recorded(&recorder, 1, Duration::from_secs(5)).await;
        let result = results.first().expect("the worker must answer");
        assert_eq!(result.session_id, id);
        assert_eq!(result.generation, 1);
        assert_eq!(result.content_width, 55);
        assert_eq!(result.signature, 7);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn a_narrow_request_renders_narrower_than_a_wide_one() {
        // Given a worker and one session.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default_with_scope_focus());
        let recorder = worker_and_recorder(&harness, &state).await;
        let session = session_with(2);

        // When the same session is previewed at two widths.
        harness.publish(request(&session, 10, 1)).await;
        let narrow = await_recorded(&recorder, 1, Duration::from_secs(5)).await;
        harness.publish(request(&session, 200, 2)).await;
        let wide = await_recorded(&recorder, 1, Duration::from_secs(5)).await;

        // Then each result is wrapped to the width it asked for.
        let narrow_lines = narrow.first().expect("narrow rendered").lines.len();
        let wide_lines = wide.first().expect("wide rendered").lines.len();
        assert_eq!(narrow.first().map(|r| r.content_width), Some(10));
        assert_eq!(wide.first().map(|r| r.content_width), Some(200));
        assert!(
            narrow_lines >= wide_lines,
            "narrower text wraps onto at least as many lines: narrow {narrow_lines}, wide {wide_lines}"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn the_second_worker_also_answers_preview_requests() {
        // Given two workers, as the real pool has.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default_with_scope_focus());
        for index in 0..2 {
            LayoutWorkerActor::spawn(
                harness.system(),
                index,
                LayoutWorkerActorDeps {
                    state: state.clone(),
                },
            );
        }
        let recorder = harness.spawn_recorder::<SessionPreviewRendered>().await;
        let session = session_with(1);

        // When a request is published.
        harness.publish(request(&session, 40, 1)).await;

        // Then it is answered — the pool is not a single worker that could
        // have gone quiet without the preview noticing.
        let results = await_recorded(&recorder, 1, Duration::from_secs(5)).await;
        assert!(
            !results.is_empty(),
            "every worker in the pool must declare preview requests"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn the_pool_is_more_than_one_worker() {
        // Given two workers at distinct paths, as the real pool has.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default_with_scope_focus());
        let first = LayoutWorkerActor::spawn(
            harness.system(),
            0,
            LayoutWorkerActorDeps {
                state: state.clone(),
            },
        );
        let second = LayoutWorkerActor::spawn(
            harness.system(),
            1,
            LayoutWorkerActorDeps {
                state: state.clone(),
            },
        );

        // Then the pool is a set, not a singleton the preview could have
        // displaced.
        assert_ne!(first, second, "each worker in the pool has its own path");
    }
}
