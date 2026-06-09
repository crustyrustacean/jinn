//! Preferences state sync actor - keeps `AppState.frontend.preferences` in sync.
//!
//! Subscribes to [`PreferencesUpdated`] events emitted by [`PreferencesActor`].
//! On each event, replaces `state.frontend.preferences` with the full payload.
//! This is the ONLY actor that writes to `frontend.preferences`.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::protocol::Event;

/// Keeps `AppState.frontend.preferences` in sync with the persisted preferences.
///
/// Subscribes to `PreferencesUpdated` events and writes the full preferences
/// to the shared state. This is the single writer for `frontend.preferences`.
pub struct PreferencesStateSyncActor {
    /// Shared application state.
    state: State,
}

/// Dependencies for [`PreferencesStateSyncActor`].
pub struct PreferencesStateSyncActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
}

impl Actor for PreferencesStateSyncActor {
    type Message = NoDirectMsg;
    type Deps = PreferencesStateSyncActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<super::protocol::event::PreferencesUpdated>();
        ctx.set_description("Syncs AppState.frontend.preferences from PreferencesUpdated events");

        Self {
            state: deps.state,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::PreferencesUpdated(ref payload)) => {
                let mut state = self.state.write();
                state.frontend.preferences = payload.preferences.clone();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::unreachable, clippy::string_slice, clippy::uninlined_format_args, reason = "test code")]
    use super::*;
    use crate::common::actor::{ActorContext, RecordingSink};
    use crate::common::services::Services;
    use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;
    use crate::feat::preferences_actor::user_preferences::UserPreferences;
    use std::sync::Arc;

    fn create_actor() -> (PreferencesStateSyncActor, ActorContext) {
        let state = State::new(crate::common::app_state::AppState::default());
        let services = Services::new();
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("prefs-state-sync", sink);
        let actor = PreferencesStateSyncActor::activate(
            PreferencesStateSyncActorDeps { state, services },
            &mut ctx,
        );
        (actor, ctx)
    }

    #[tokio::test]
    async fn preferences_updated_syncs_to_frontend() {
        // Given a sync actor and a PreferencesUpdated event with custom preferences.
        let (mut actor, ctx) = create_actor();
        let prefs = UserPreferences::default();
        let event = PreferencesUpdated {
            preferences: prefs.clone(),
        };

        // When handling the PreferencesUpdated event.
        actor
            .handle(
                ActorEnvelope::Event(Event::PreferencesUpdated(event)),
                &ctx,
            )
            .await;

        // Then the frontend preferences match.
        let guard = actor.state.read();
        assert_eq!(guard.frontend.preferences, prefs);
    }

    #[tokio::test]
    async fn ignores_unrelated_events() {
        // Given a sync actor.
        let (mut actor, ctx) = create_actor();

        // When handling an unrelated event.
        actor
            .handle(ActorEnvelope::Event(Event::AppStateUpdated(
                crate::feat::preferences_actor::protocol::app_state_event::AppStateUpdated {
                    state: crate::feat::preferences_actor::app_state_file::AppStateFile::default(),
                },
            )), &ctx)
            .await;


        // Then the frontend preferences remain default.
        let guard = actor.state.read();
        assert_eq!(guard.frontend.preferences, UserPreferences::default());
    }
}
