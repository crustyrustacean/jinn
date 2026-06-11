//! App-state actor - persists runtime state to `state.toml`.
//!
//! Subscribes to [`UpdateAppState`] commands carrying batches of
//! [`AppStateUpdate`] diffs. On each command, loads current state,
//! applies all diffs, saves to disk, and emits an [`AppStateUpdated`]
//! event with the full result.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::feat::preferences_actor::protocol::app_state_command::UpdateAppState;
use crate::feat::preferences_actor::protocol::app_state_event::AppStateUpdated;
use crate::feat::session::model_selection::ModelSelection;

use crate::protocol::{Command, Event};

/// The app-state actor.
///
/// Subscribes to `UpdateAppState` commands and persists state
/// diffs to `state.toml`, then emits `AppStateUpdated` so
/// downstream actors can sync their caches.
pub struct AppStateActor {
    /// Runtime services.
    services: Services,
}

/// Dependencies for [`AppStateActor`].
pub struct AppStateActorDeps {
    /// Runtime services.
    pub services: Services,
}

impl Actor for AppStateActor {
    type Message = NoDirectMsg;
    type Deps = AppStateActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<UpdateAppState>();
        ctx.set_description("Persists runtime state to state.toml");

        Self {
            services: deps.services,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(Command::UpdateAppState(ref payload)) => {
                self.handle_update_app_state(payload, ctx);
            }
            ActorEnvelope::Event(_) | ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }
}

impl AppStateActor {
    /// Processes a batch of state diffs: load, apply, save, emit.
    fn handle_update_app_state(&self, payload: &UpdateAppState, ctx: &ActorContext) {
        let mut state = self.services.app_state_storage.read();
        for update in &payload.updates {
            update.apply(&mut state);
        }
        if let Err(e) = self.services.app_state_storage.save(&state) {
            tracing::warn!(err = ?e, "app-state-actor failed to save app state");
            return;
        }
        if let Err(e) = ctx.send_event(Event::AppStateUpdated(AppStateUpdated { state })) {
            tracing::warn!(err = ?e, "app-state-actor failed to emit AppStateUpdated");
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use std::sync::Arc;

    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::common::services::Services;

    use crate::feat::preferences_actor::app_state_storage::InMemoryAppStateStorage;
    use crate::feat::preferences_actor::protocol::app_state_command::AppStateUpdate;
    use crate::feat::preferences_actor::protocol::app_state_command::UpdateAppState;
    use crate::feat::session::model_selection::ModelSelection;
    use crate::protocol::Command;

    use super::{AppStateActor, AppStateActorDeps};

    fn create_actor() -> (AppStateActor, Arc<RecordingSink>, Services, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("app-state", sink.clone() as Arc<dyn MessageSink>);

        let storage = InMemoryAppStateStorage::new();
        let mut services = Services::new();
        let svc = crate::feat::preferences_actor::app_state_storage::AppStateStorageService::new(
            Arc::new(storage),
        );
        svc.reload().expect("test app state storage initial reload");
        services.app_state_storage = svc;

        let deps = AppStateActorDeps {
            services: services.clone(),
        };
        let actor = AppStateActor::activate(deps, &mut ctx);
        (actor, sink, services, ctx)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn set_last_model_persists_and_emits() {
        // Given an app-state actor.
        let (mut actor, sink, services, ctx) = create_actor();

        // When handling UpdateAppState with SetLastModel.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdateAppState(UpdateAppState {
                    updates: vec![AppStateUpdate::SetLastModel(Some(
                        ModelSelection::from_single("anthropic/claude-sonnet-4".to_owned()),
                    ))],
                })),
                &ctx,
            )
            .await;

        // Then the storage has the last model.
        let loaded = services.app_state_storage.read();
        let expected = ModelSelection::from_single("anthropic/claude-sonnet-4".to_owned());
        assert_eq!(loaded.last_model, Some(expected));

        // And an AppStateUpdated event was emitted.
        let events = sink.events();
        assert_eq!(events.len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn set_theme_persists_and_emits() {
        // Given an app-state actor.
        let (mut actor, sink, services, ctx) = create_actor();

        // When handling UpdateAppState with SetTheme.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdateAppState(UpdateAppState {
                    updates: vec![AppStateUpdate::SetTheme(Some("dracula".to_owned()))],
                })),
                &ctx,
            )
            .await;

        // Then the storage has the theme.
        let loaded = services.app_state_storage.read();
        assert_eq!(loaded.theme_name.as_deref(), Some("dracula"));

        // And an AppStateUpdated event was emitted.
        let events = sink.events();
        assert_eq!(events.len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn ignores_unrelated_commands() {
        // Given an app-state actor.
        let (mut actor, sink, services, ctx) = create_actor();

        // When handling an unrelated command.
        actor
            .handle(ActorEnvelope::Command(Command::RefreshModels), &ctx)
            .await;

        // Then no events were emitted.
        assert!(sink.events().is_empty());

        // And storage still has defaults.
        let loaded = services.app_state_storage.read();
        assert!(loaded.last_model.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn multiple_updates_in_one_command() {
        // Given an app-state actor.
        let (mut actor, _sink, services, ctx) = create_actor();

        // When handling a batch with multiple updates.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdateAppState(UpdateAppState {
                    updates: vec![
                        AppStateUpdate::SetLastModel(Some(ModelSelection::from_single(
                            "openrouter/gpt-4".to_owned(),
                        ))),
                        AppStateUpdate::SetSidebarWidth(Some(40)),
                        AppStateUpdate::SetTheme(Some("nord".to_owned())),
                    ],
                })),
                &ctx,
            )
            .await;

        // Then all three fields are persisted.
        let loaded = services.app_state_storage.read();
        let expected = ModelSelection::from_single("openrouter/gpt-4".to_owned());
        assert_eq!(loaded.last_model, Some(expected));
        assert_eq!(loaded.sidebar_width, Some(40));
        assert_eq!(loaded.theme_name.as_deref(), Some("nord"));
    }
}
