//! Sidebar state actor — keeps sidebar cursor in sync after session close.
//!
//! Subscribes to [`SessionClosed`] events and clamps the sidebar's
//! `selected_index` and `scroll_offset` so they never point past the end
//! of the sessions list.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::session::protocol::session_closed::SessionClosed;
use crate::feat::ui::sidebar::sessions;
use crate::protocol::Event;

/// Actor that adjusts sidebar cursor state in response to session close.
pub struct SidebarStateActor {
    state: State,
}

/// Dependencies for [`SidebarStateActor`].
pub struct SidebarStateActorDeps {
    /// Shared application state.
    pub state: State,
}

impl Actor for SidebarStateActor {
    type Message = NoDirectMsg;
    type Deps = SidebarStateActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<SessionClosed>();
        ctx.set_description("Sidebar cursor state management");

        Self { state: deps.state }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        if let ActorEnvelope::Event(Event::SessionClosed(payload)) = &msg {
            self.handle_session_closed(payload);
        }
    }
}

impl SidebarStateActor {
    /// Reconcile sidebar cursor and active session after a session is closed.
    fn handle_session_closed(&self, _payload: &SessionClosed) {
        let mut state = self.state.write();
        sessions::reconcile_after_session_removal(&mut state);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::session::chat_session::ChatSessionState;

    fn test_actor() -> SidebarStateActor {
        SidebarStateActor {
            state: State::new(AppState::default()),
        }
    }

    #[test]
    fn clamps_selected_index_after_session_removed() {
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
            state.frontend.sessions_section.selected_index = Some(2);
            id3
        };

        // Simulate the session being removed (as the session actor would do).
        {
            let mut state = actor.state.write();
            state.session.remove_without_replacement(&removed_id);
        }

        // When handling SessionClosed.
        let payload = crate::feat::session::protocol::session_closed::SessionClosed {
            session_id: removed_id,
        };
        actor.handle_session_closed(&payload);

        // Then selected_index is clamped to 1 (max valid index).
        let state = actor.state.read();
        assert_eq!(state.frontend.sessions_section.selected_index, Some(1));
    }

    #[test]
    fn handles_removal_of_last_session_cursor_at_zero() {
        // Given a sidebar actor with one session and cursor at 0.
        let actor = test_actor();
        let removed_id = {
            let mut state = actor.state.write();
            let id = state.session.active_session_id().clone();
            state.frontend.sessions_section.selected_index = Some(0);
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
        let payload = crate::feat::session::protocol::session_closed::SessionClosed {
            session_id: removed_id,
        };
        actor.handle_session_closed(&payload);

        // Then cursor stays at 0.
        let state = actor.state.read();
        assert_eq!(state.frontend.sessions_section.selected_index, Some(0));
    }

    #[test]
    fn cursor_stays_when_index_still_valid() {
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
            state.frontend.sessions_section.selected_index = Some(0);
            id3
        };

        // Simulate removal of the last session (cursor at 0 is still valid).
        {
            let mut state = actor.state.write();
            state.session.remove_without_replacement(&removed_id);
        }

        // When handling SessionClosed.
        let payload = crate::feat::session::protocol::session_closed::SessionClosed {
            session_id: removed_id,
        };
        actor.handle_session_closed(&payload);

        // Then cursor stays at 0.
        let state = actor.state.read();
        assert_eq!(state.frontend.sessions_section.selected_index, Some(0));
    }
}
