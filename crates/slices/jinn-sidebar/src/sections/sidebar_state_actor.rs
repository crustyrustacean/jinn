//! Sidebar state actor — keeps sidebar cursor in sync after session removal.
//!
//! A trouper [`ServiceActor`] subscribed to the slice's `jinn.sidebar`
//! topic (fed by the kernel bridge's forward routes). It folds
//! [`SessionRemoved`] into the sidebar cursor and active session.

use trouper::actor::ActorPath;
use trouper::actor::{MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

use crate::sections::sessions;
use jinn_domain::common::state::State;
use jinn_session_msg::{SessionArchiveFailed, SessionRemoved, SessionTeardownFinished};

/// The sidebar state actor's static trouper path.
pub const SIDEBAR_STATE_PATH: &str = "sidebar-state";

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
            .handles::<SessionArchiveFailed>()
            .handles::<SessionTeardownFinished>()
            .start()
    }

    /// Reconcile sidebar cursor and active session after a session is removed.
    fn handle_session_removed(&self, payload: &SessionRemoved) {
        self.state.with_session_sidebar(|view| {
            sessions::state::clear_in_flight(view.frontend, &payload.session_id);
            sessions::state::repair_visual_parents_after_removal(
                view.session.map(),
                view.frontend,
                &payload.session_id,
                payload.removed_parent.as_ref(),
            );
            sessions::reconcile_split(view.session.map(), view.frontend);
        });
    }

    /// Stops indicating an archive that did not complete, leaving the row live.
    fn handle_session_archive_failed(&self, payload: &SessionArchiveFailed) {
        self.state.with_session_sidebar(|view| {
            sessions::state::clear_in_flight(view.frontend, &payload.session_id);
        });
    }

    /// Stops indicating a teardown that failed, leaving the row live.
    ///
    /// A *successful* teardown deliberately keeps the tint: it is immediately
    /// followed by the archive write, and that write is the work worth showing.
    fn handle_session_teardown_finished(&self, payload: &SessionTeardownFinished) {
        if payload.error.is_none() {
            return;
        }
        self.state.with_session_sidebar(|view| {
            sessions::state::clear_in_flight(view.frontend, &payload.session_id);
        });
    }
}

impl MsgHandler<SessionRemoved> for SidebarStateActor {
    async fn handle(&mut self, msg: &SessionRemoved, _ctx: &mut MsgCtx<'_>) {
        self.handle_session_removed(msg);
    }
}

impl MsgHandler<SessionArchiveFailed> for SidebarStateActor {
    async fn handle(&mut self, msg: &SessionArchiveFailed, _ctx: &mut MsgCtx<'_>) {
        self.handle_session_archive_failed(msg);
    }
}

impl MsgHandler<SessionTeardownFinished> for SidebarStateActor {
    async fn handle(&mut self, msg: &SessionTeardownFinished, _ctx: &mut MsgCtx<'_>) {
        self.handle_session_teardown_finished(msg);
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

    /// Marks a session in flight and returns the actor plus the session id.
    fn actor_with_in_flight_session() -> (SidebarStateActor, jinn_core_types::SessionId) {
        let actor = test_actor();
        let id = jinn_core_types::SessionId::new();
        actor
            .state
            .write()
            .frontend
            .update_sections(|s| s.sessions.begin_in_flight(std::slice::from_ref(&id)));
        (actor, id)
    }

    /// Whether the given session is still marked in flight.
    fn still_in_flight(actor: &SidebarStateActor, id: &jinn_core_types::SessionId) -> bool {
        actor
            .state
            .read()
            .frontend
            .with_sections(|s| s.sessions.is_in_flight(id), || false)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_removed_clears_in_flight() {
        // Given a session marked in flight.
        let (actor, id) = actor_with_in_flight_session();

        // When handling SessionRemoved for it.
        actor.handle_session_removed(&SessionRemoved {
            session_id: id.clone(),
            removed_parent: None,
        });

        // Then the in-flight mark is cleared.
        assert!(!still_in_flight(&actor, &id));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn archive_failure_clears_in_flight() {
        // Given a session marked in flight.
        let (actor, id) = actor_with_in_flight_session();

        // When handling a SessionArchiveFailed for it.
        actor.handle_session_archive_failed(&SessionArchiveFailed {
            session_id: id.clone(),
            error: "write failed".to_owned(),
        });

        // Then the in-flight mark is cleared.
        assert!(!still_in_flight(&actor, &id));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn failed_teardown_clears_in_flight() {
        // Given a session marked in flight.
        let (actor, id) = actor_with_in_flight_session();

        // When handling a teardown that reported an error.
        actor.handle_session_teardown_finished(&SessionTeardownFinished {
            session_id: id.clone(),
            error: Some("script failed".to_owned()),
        });

        // Then the in-flight mark is cleared.
        assert!(!still_in_flight(&actor, &id));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn successful_teardown_keeps_in_flight() {
        // Given a session marked in flight.
        let (actor, id) = actor_with_in_flight_session();

        // When handling a teardown that reported no error.
        actor.handle_session_teardown_finished(&SessionTeardownFinished {
            session_id: id.clone(),
            error: None,
        });

        // Then the mark survives, because the archive write follows it.
        assert!(still_in_flight(&actor, &id));
    }

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
