//! App-state actor - persists runtime state to `state.toml`.
//!
//! Subscribes to [`UpdateAppState`] commands carrying batches of
//! [`AppStateUpdate`] diffs. On each command, loads current state,
//! applies all diffs, saves to disk, and emits an [`AppStateUpdated`]
//! event with the full result.

use std::path::PathBuf;

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::state::State;
use crate::feat::preferences_actor::app_state_file::AppStateFile;
use crate::feat::preferences_actor::protocol::app_state_command::UpdateAppState;
use crate::feat::preferences_actor::protocol::app_state_event::AppStateUpdated;
use crate::feat::theme;

/// Dependencies for spawning an [`AppStateActor`].
#[derive(Clone)]
pub struct AppStateActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
    /// Shared application state.
    pub state: State,
}

/// The app-state actor.
///
/// Subscribes to `UpdateAppState` commands and persists state
/// diffs to `state.toml`, then emits `AppStateUpdated` so
/// downstream actors can sync their caches.
pub struct AppStateActor {
    deps: ActorDeps,
    /// Shared application state — writes frontend.app_state, sidebar_width,
    /// theme, and context.active_persona inline after persist.
    state: State,
    themes_dir: PathBuf,
    system_themes_dir: PathBuf,
}

impl Actor for AppStateActor {
    type Args = AppStateActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<UpdateAppState>())
            .await;

        Ok(Self {
            deps: args.deps.clone(),
            state: args.state,
            themes_dir: args.deps.services.paths.themes_dir(),
            system_themes_dir: args.deps.services.paths.system_themes_dir(),
        })
    }
}

impl AppStateActor {
    /// Apply state updates, persist, and emit event.
    pub(crate) async fn handle_update(&mut self, msg: UpdateAppState) {
        let mut state = self.deps.services.app_state_storage.read();
        for update in &msg.updates {
            update.apply(&mut state);
        }
        if let Err(e) = self.deps.services.app_state_storage.save(&state) {
            tracing::warn!(err = ?e, "app-state-actor failed to save app state");
            return;
        }
        // Write frontend/context fields inline after persist.
        self.sync_state(&state);
        self.publish(AppStateUpdated { state }).await;
    }

    /// Syncs persisted state into the shared `AppState` frontend/context fields.
    fn sync_state(&self, updated: &AppStateFile) {
        let mut state = self.state.write();

        // Cache the entire state for runtime access.
        state.frontend.app_state = updated.clone();

        state.frontend.sidebar_width = updated.sidebar_width.unwrap_or(30);

        // Reload theme when theme_name changes.
        match theme::resolve_theme(
            updated.theme_name.as_deref(),
            &self.themes_dir,
            &self.system_themes_dir,
        ) {
            Ok(t) => {
                state.frontend.theme = t;
                state.invalidate_theme_caches();
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to reload theme, keeping current");
            }
        }

        // Sync active_persona when persona_name changes.
        if let Some(ref persona_name) = updated.persona_name {
            let found = state
                .context
                .personas()
                .iter()
                .find(|p| p.name == *persona_name)
                .cloned();
            if let Some(persona) = found {
                state.context.set_active_persona(Some(persona));
            }
        }
    }
}

impl Message<UpdateAppState> for AppStateActor {
    type Reply = ();

    async fn handle(&mut self, msg: UpdateAppState, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_update(msg).await;
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

    use super::AppStateActor;
    use crate::common::actor_deps::ActorDeps;
    use crate::common::services::Services;
    use crate::common::services::bus_service::BusAudit;
    use crate::feat::preferences_actor::app_state_file::AppStateFile;
    use crate::feat::preferences_actor::app_state_storage::InMemoryAppStateStorage;
    use crate::feat::preferences_actor::protocol::app_state_command::{
        AppStateUpdate, UpdateAppState,
    };
    use crate::feat::preferences_actor::protocol::app_state_event::AppStateUpdated;
    use crate::feat::session::model_selection::ModelSelection;
    async fn create_actor() -> (AppStateActor, BusAudit, Services) {
        let (bus, audit) = crate::common::services::BusService::new_recording();
        let mut services = Services::new_fake_with_bus(bus).await;

        let storage = InMemoryAppStateStorage::new();
        let svc = crate::feat::preferences_actor::app_state_storage::AppStateStorageService::new(
            Arc::new(storage),
        );
        svc.reload().expect("test app state storage initial reload");
        services.app_state_storage = svc;

        let actor = AppStateActor {
            deps: ActorDeps {
                services: services.clone(),
            },
            state: crate::common::state::State::new(crate::common::app_state::AppState::default()),
            themes_dir: std::path::PathBuf::new(),
            system_themes_dir: std::path::PathBuf::new(),
        };
        (actor, audit, services)
    }

    #[tokio::test]
    async fn set_last_model_persists_and_emits() {
        // Given an app-state actor.
        let (mut actor, audit, services) = create_actor().await;

        // When handling UpdateAppState with SetLastModel.
        actor
            .handle_update(UpdateAppState {
                updates: vec![AppStateUpdate::SetLastModel(Some(
                    ModelSelection::from_single("anthropic/claude-sonnet-4".to_owned()),
                ))],
            })
            .await;

        // Then the storage has the last model.
        let loaded = services.app_state_storage.read();
        let expected = ModelSelection::from_single("anthropic/claude-sonnet-4".to_owned());
        assert_eq!(loaded.last_model, Some(expected));

        // And an AppStateUpdated event was emitted.
        let events = audit.of_type::<AppStateUpdated>();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn set_theme_persists_and_emits() {
        // Given an app-state actor.
        let (mut actor, audit, services) = create_actor().await;

        // When handling UpdateAppState with SetTheme.
        actor
            .handle_update(UpdateAppState {
                updates: vec![AppStateUpdate::SetTheme(Some("dracula".to_owned()))],
            })
            .await;

        // Then the storage has the theme.
        let loaded = services.app_state_storage.read();
        assert_eq!(loaded.theme_name.as_deref(), Some("dracula"));

        // And an AppStateUpdated event was emitted.
        let events = audit.of_type::<AppStateUpdated>();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn multiple_updates_in_one_command() {
        // Given an app-state actor.
        let (mut actor, _audit, services) = create_actor().await;

        // When handling a batch with multiple updates.
        actor
            .handle_update(UpdateAppState {
                updates: vec![
                    AppStateUpdate::SetLastModel(Some(ModelSelection::from_single(
                        "openrouter/gpt-4".to_owned(),
                    ))),
                    AppStateUpdate::SetSidebarWidth(Some(40)),
                    AppStateUpdate::SetTheme(Some("nord".to_owned())),
                ],
            })
            .await;

        // Then all three fields are persisted.
        let loaded = services.app_state_storage.read();
        let expected = ModelSelection::from_single("openrouter/gpt-4".to_owned());
        assert_eq!(loaded.last_model, Some(expected));
        assert_eq!(loaded.sidebar_width, Some(40));
        assert_eq!(loaded.theme_name.as_deref(), Some("nord"));
    }

    #[tokio::test]
    async fn sync_state_sets_sidebar_width_default_30() {
        // Given an app-state actor.
        let (actor, _audit, _services) = create_actor().await;

        // When syncing AppStateFile with sidebar_width = None.
        let app_state = AppStateFile {
            sidebar_width: None,
            ..AppStateFile::default()
        };
        actor.sync_state(&app_state);

        // Then sidebar_width is the default 30.
        let guard = actor.state.read();
        assert_eq!(guard.frontend.sidebar_width, 30);
    }

    #[tokio::test]
    async fn sync_state_updates_sidebar_width() {
        // Given an app-state actor.
        let (actor, _audit, _services) = create_actor().await;

        // When syncing AppStateFile with sidebar_width = 50.
        let app_state = AppStateFile {
            sidebar_width: Some(50),
            ..AppStateFile::default()
        };
        actor.sync_state(&app_state);

        // Then sidebar_width is 50.
        let guard = actor.state.read();
        assert_eq!(guard.frontend.sidebar_width, 50);
    }

    #[tokio::test]
    async fn sync_state_sets_correct_persona() {
        use crate::feat::persona::Persona;
        // If the condition were flipped, the wrong persona would be set.
        // Given an app-state actor with two personas loaded.
        let (actor, _audit, _services) = create_actor().await;
        {
            let mut guard = actor.state.write();
            guard.context.personas = vec![
                Persona {
                    name: "coder".to_owned(),
                    description: String::new(),
                    body: String::new(),
                    file_path: std::path::PathBuf::new(),
                },
                Persona {
                    name: "writer".to_owned(),
                    description: String::new(),
                    body: String::new(),
                    file_path: std::path::PathBuf::new(),
                },
            ];
        }

        // When syncing AppStateFile with persona_name = "writer".
        let app_state = AppStateFile {
            persona_name: Some("writer".to_owned()),
            ..AppStateFile::default()
        };
        actor.sync_state(&app_state);

        // Then the active persona is "writer", not "coder".
        let guard = actor.state.read();
        let active = guard
            .context
            .active_persona
            .as_ref()
            .expect("should have active persona");
        assert_eq!(active.name, "writer");
    }

    #[tokio::test]
    async fn sync_state_resolves_default_theme_when_none() {
        // Given an app-state actor.
        let (actor, _audit, _services) = create_actor().await;

        // When syncing AppStateFile with theme_name = None.
        let app_state = AppStateFile {
            theme_name: None,
            ..AppStateFile::default()
        };
        actor.sync_state(&app_state);

        // Then the theme was resolved and caches invalidated without panic.
        // resolve_theme(None, ...) returns the embedded default theme.
        let _guard = actor.state.read();
        // If we reach here, the handler completed successfully.
    }
}
