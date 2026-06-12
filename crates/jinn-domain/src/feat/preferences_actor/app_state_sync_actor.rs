//! App state sync actor — keeps `AppState.frontend` in sync with persisted state.
//!
//! Subscribes to [`AppStateUpdated`] events emitted by [`AppStateActor`].
//! On each event, syncs `sidebar_width`, reloads the theme, and sets the
//! active persona. This is the ONLY actor that writes to `frontend.sidebar_width`
//! based on state-file changes (the intent handler also writes it directly
//! for immediate feedback).

use std::path::PathBuf;

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::common::actor_deps::ActorDeps;
use crate::common::state::State;
use crate::feat::preferences_actor::protocol::app_state_event::AppStateUpdated;
use crate::feat::theme;

/// Keeps `AppState.frontend` in sync with persisted app state.
///
/// Subscribes to `AppStateUpdated` events and writes state-derived fields
/// (sidebar width, theme, active persona) to the shared state.
pub struct AppStateSyncActor {
    /// Shared application state.
    state: State,
    /// Path to the user themes directory.
    themes_dir: PathBuf,
    /// Path to the system themes directory.
    system_themes_dir: PathBuf,
}

/// Dependencies for spawning an [`AppStateSyncActor`].
pub struct AppStateSyncActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
    /// Shared application state.
    pub state: State,
}

impl Actor for AppStateSyncActor {
    type Args = AppStateSyncActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<AppStateUpdated>())
            .await;

        Ok(Self {
            state: args.state,
            themes_dir: args.deps.services.paths.themes_dir(),
            system_themes_dir: args.deps.services.paths.system_themes_dir(),
        })
    }
}

impl Message<AppStateUpdated> for AppStateSyncActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: AppStateUpdated,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.sync_state(&msg);
    }
}

impl AppStateSyncActor {
    /// Applies the updated state to the shared application state.
    fn sync_state(&self, msg: &AppStateUpdated) {
        let mut state = self.state.write();
        let updated = &msg.state;

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
                .personas
                .iter()
                .find(|p| p.name == *persona_name)
                .cloned();
            if let Some(persona) = found {
                state.context.active_persona = Some(persona);
            }
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

    use std::path::PathBuf;

    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::persona::Persona;
    use crate::feat::preferences_actor::app_state_file::AppStateFile;
    use crate::feat::preferences_actor::protocol::app_state_event::AppStateUpdated;

    use super::AppStateSyncActor;

    fn create_actor() -> (AppStateSyncActor, State) {
        let state = State::new(AppState::default());
        let actor = AppStateSyncActor {
            state: state.clone(),
            themes_dir: PathBuf::new(),
            system_themes_dir: PathBuf::new(),
        };
        (actor, state)
    }

    #[rstest::rstest]
    fn sidebar_width_defaults_to_30_when_none() {
        // Given a sync actor.
        let (actor, state) = create_actor();

        // When receiving AppStateUpdated with sidebar_width = None.
        let app_state = AppStateFile {
            sidebar_width: None,
            ..AppStateFile::default()
        };
        actor.sync_state(&AppStateUpdated { state: app_state });

        // Then sidebar_width is the default 30.
        let guard = state.read();
        assert_eq!(guard.frontend.sidebar_width, 30);
    }

    #[rstest::rstest]
    fn sidebar_width_updates_from_state() {
        // Given a sync actor.
        let (actor, state) = create_actor();

        // When receiving AppStateUpdated with sidebar_width = 50.
        let app_state = AppStateFile {
            sidebar_width: Some(50),
            ..AppStateFile::default()
        };
        actor.sync_state(&AppStateUpdated { state: app_state });

        // Then sidebar_width is 50.
        let guard = state.read();
        assert_eq!(guard.frontend.sidebar_width, 50);
    }

    #[rstest::rstest]
    fn persona_name_sync_sets_correct_persona() {
        // Kills: replace == with != in persona_name matching.
        // If the condition were flipped, the wrong persona would be set.
        // Given a sync actor with two personas loaded.
        let (actor, state) = create_actor();
        {
            let mut guard = state.write();
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

        // When receiving AppStateUpdated with persona_name = "writer".
        let app_state = AppStateFile {
            persona_name: Some("writer".to_owned()),
            ..AppStateFile::default()
        };
        actor.sync_state(&AppStateUpdated { state: app_state });

        // Then the active persona is "writer", not "coder".
        let guard = state.read();
        let active = guard
            .context
            .active_persona
            .as_ref()
            .expect("should have active persona");
        assert_eq!(active.name, "writer");
    }

    #[rstest::rstest]
    fn theme_name_none_resolves_default_theme() {
        // Given a sync actor.
        let (actor, state) = create_actor();

        // When receiving AppStateUpdated with theme_name = None.
        let app_state = AppStateFile {
            theme_name: None,
            ..AppStateFile::default()
        };
        actor.sync_state(&AppStateUpdated { state: app_state });

        // Then the theme was resolved and caches invalidated without panic.
        // resolve_theme(None, ...) returns the embedded default theme.
        let _guard = state.read();
        // If we reach here, the handler completed successfully.
    }
}
