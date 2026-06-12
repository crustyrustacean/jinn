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
    use std::time::Duration;

    use super::{AppStateActor, AppStateActorDeps};
    use crate::common::actor_deps::ActorDeps;
    use crate::common::bus::test_harness::{TestHarness, await_recorded};
    use crate::common::services::Services;
    use crate::feat::preferences_actor::app_state_storage::InMemoryAppStateStorage;
    use crate::feat::preferences_actor::protocol::app_state_command::AppStateUpdate;
    use crate::feat::preferences_actor::protocol::app_state_command::UpdateAppState;
    use crate::feat::preferences_actor::protocol::app_state_event::AppStateUpdated;

    async fn create_harness() -> (TestHarness, Services) {
        let harness = TestHarness::new().await;
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
        // Given an app-state actor and a recorder for AppStateUpdated.
        let (harness, services) = create_harness().await;
        let _actor = harness
            .spawn_actor::<AppStateActor>(AppStateActorDeps {
                deps: ActorDeps {
                    services: services.clone(),
                },
            })
            .await;
        let recorder = harness.spawn_recorder::<AppStateUpdated>().await;

        // When publishing UpdateAppState with SetLastModel.
        harness
            .publish(UpdateAppState {
                updates: vec![AppStateUpdate::SetLastModel(Some(
                    "anthropic/claude-sonnet-4".to_owned(),
                ))],
            })
            .await;

        // Then an AppStateUpdated event was emitted.
        let events = await_recorded(&recorder, 1, Duration::from_secs(1)).await;
        assert_eq!(events.len(), 1);

        // And the storage has the last model.
        let loaded = services.app_state_storage.read();
        assert_eq!(
            loaded.last_model.as_deref(),
            Some("anthropic/claude-sonnet-4")
        );
    }

    #[tokio::test]
    async fn set_theme_persists_and_emits() {
        // Given an app-state actor and a recorder for AppStateUpdated.
        let (harness, services) = create_harness().await;
        let _actor = harness
            .spawn_actor::<AppStateActor>(AppStateActorDeps {
                deps: ActorDeps {
                    services: services.clone(),
                },
            })
            .await;
        let recorder = harness.spawn_recorder::<AppStateUpdated>().await;

        // When publishing UpdateAppState with SetTheme.
        harness
            .publish(UpdateAppState {
                updates: vec![AppStateUpdate::SetTheme(Some("dracula".to_owned()))],
            })
            .await;

        // Then an AppStateUpdated event was emitted.
        let events = await_recorded(&recorder, 1, Duration::from_secs(1)).await;
        assert_eq!(events.len(), 1);

        // And the storage has the theme.
        let loaded = services.app_state_storage.read();
        assert_eq!(loaded.theme_name.as_deref(), Some("dracula"));
    }

    #[tokio::test]
    async fn multiple_updates_in_one_command() {
        // Given an app-state actor and a recorder for AppStateUpdated.
        let (harness, services) = create_harness().await;
        let _actor = harness
            .spawn_actor::<AppStateActor>(AppStateActorDeps {
                deps: ActorDeps {
                    services: services.clone(),
                },
            })
            .await;
        let recorder = harness.spawn_recorder::<AppStateUpdated>().await;

        // When publishing a batch with multiple updates.
        harness
            .publish(UpdateAppState {
                updates: vec![
                    AppStateUpdate::SetLastModel(Some("openrouter/gpt-4".to_owned())),
                    AppStateUpdate::SetSidebarWidth(Some(40)),
                    AppStateUpdate::SetTheme(Some("nord".to_owned())),
                ],
            })
            .await;

        // Then an AppStateUpdated event was emitted.
        let events = await_recorded(&recorder, 1, Duration::from_secs(1)).await;
        assert_eq!(events.len(), 1);

        // And all three fields are persisted.
        let loaded = services.app_state_storage.read();
        assert_eq!(loaded.last_model.as_deref(), Some("openrouter/gpt-4"));
        assert_eq!(loaded.sidebar_width, Some(40));
        assert_eq!(loaded.theme_name.as_deref(), Some("nord"));
    }
}
