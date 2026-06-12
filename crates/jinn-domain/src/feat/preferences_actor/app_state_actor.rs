//! App-state actor - persists runtime state to `state.toml`.
//!
//! Subscribes to [`UpdateAppState`] commands carrying batches of
//! [`AppStateUpdate`] diffs. On each command, loads current state,
//! applies all diffs, saves to disk, and emits an [`AppStateUpdated`]
//! event with the full result.

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::feat::preferences_actor::protocol::app_state_command::UpdateAppState;
use crate::feat::preferences_actor::protocol::app_state_event::AppStateUpdated;
use crate::feat::session::model_selection::ModelSelection;

/// Dependencies for spawning an [`AppStateActor`].
pub struct AppStateActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
}

/// The app-state actor.
///
/// Subscribes to `UpdateAppState` commands and persists state
/// diffs to `state.toml`, then emits `AppStateUpdated` so
/// downstream actors can sync their caches.
pub struct AppStateActor {
    deps: ActorDeps,
}

impl Actor for AppStateActor {
    type Args = AppStateActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<UpdateAppState>())
            .await;

        Ok(Self { deps: args.deps })
    }
}

impl Message<UpdateAppState> for AppStateActor {
    type Reply = ();

    async fn handle(&mut self, msg: UpdateAppState, _ctx: &mut Context<Self, Self::Reply>) {
        let mut state = self.deps.services.app_state_storage.read();
        for update in &msg.updates {
            update.apply(&mut state);
        }
        if let Err(e) = self.deps.services.app_state_storage.save(&state) {
            tracing::warn!(err = ?e, "app-state-actor failed to save app state");
            return;
        }
        self.publish(AppStateUpdated { state }).await;
    }
}

impl BusPublish for AppStateActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        self.deps.bus()
    }
}

//FIXME: plugin migration
#[cfg(any())]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use std::sync::Arc;
    use std::time::Duration;

    use super::{AppStateActor, AppStateActorDeps};
    use crate::common::actor_deps::ActorDeps;
    use crate::common::bus::test_harness::{TestHarness, await_recorded};
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
        let mut services = Services::new_fake().await;
        let svc = crate::feat::preferences_actor::app_state_storage::AppStateStorageService::new(
            Arc::new(storage),
        );
        svc.reload().expect("test app state storage initial reload");
        services.app_state_storage = svc;
        services.bus = harness.bus();
        (harness, services)
    }

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

        // Then an AppStateUpdated event was emitted.
        let events = await_recorded(&recorder, 1, Duration::from_secs(1)).await;
        assert_eq!(events.len(), 1);

        // And the storage has the last model.
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
