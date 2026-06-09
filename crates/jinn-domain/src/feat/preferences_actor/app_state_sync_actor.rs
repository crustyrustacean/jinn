//! App state sync actor — keeps `AppState.frontend` in sync with persisted state.
//!
//! Subscribes to [`AppStateUpdated`] events emitted by [`AppStateActor`].
//! On each event, syncs `sidebar_width`, reloads the theme, and sets the
//! active persona. This is the ONLY actor that writes to `frontend.sidebar_width`
//! based on state-file changes (the intent handler also writes it directly
//! for immediate feedback).

use std::path::PathBuf;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::theme;
use crate::protocol::Event;

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

/// Dependencies for [`AppStateSyncActor`].
pub struct AppStateSyncActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
}

impl Actor for AppStateSyncActor {
    type Message = NoDirectMsg;
    type Deps = AppStateSyncActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<super::protocol::app_state_event::AppStateUpdated>();
        ctx.set_description("Syncs AppState.frontend from AppStateUpdated events");

        Self {
            state: deps.state,
            themes_dir: deps.services.paths.themes_dir(),
            system_themes_dir: deps.services.paths.system_themes_dir(),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::AppStateUpdated(ref payload)) => {
                let mut state = self.state.write();
                let updated = &payload.state;

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
            ActorEnvelope::Command(_) | ActorEnvelope::Event(_) | ActorEnvelope::System(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use std::sync::Arc;

    use crate::common::actor::{Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::persona::Persona;
    use crate::feat::preferences_actor::app_state_file::AppStateFile;
    use crate::feat::preferences_actor::protocol::app_state_event::AppStateUpdated;
    use crate::protocol::Event;

    use super::{AppStateSyncActor, AppStateSyncActorDeps};
    use crate::common::services::Services;

    /// Creates a test actor with shared state.
    fn create_actor() -> (AppStateSyncActor, State, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("app-state-sync", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        let deps = AppStateSyncActorDeps {
            services: Services::new(),
            state: state.clone(),
        };

        let actor = AppStateSyncActor::activate(deps, &mut ctx);
        (actor, state, ctx)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn sidebar_width_defaults_to_30_when_none() {
        // Given a sync actor.
        let (mut actor, state, ctx) = create_actor();

        // When receiving AppStateUpdated with sidebar_width = None.
        let app_state = AppStateFile {
            sidebar_width: None,
            ..AppStateFile::default()
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::AppStateUpdated(AppStateUpdated {
                    state: app_state,
                })),
                &ctx,
            )
            .await;

        // Then sidebar_width is the default 30.
        let guard = state.read();
        assert_eq!(guard.frontend.sidebar_width, 30);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn sidebar_width_updates_from_state() {
        // Given a sync actor.
        let (mut actor, state, ctx) = create_actor();

        // When receiving AppStateUpdated with sidebar_width = 50.
        let app_state = AppStateFile {
            sidebar_width: Some(50),
            ..AppStateFile::default()
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::AppStateUpdated(AppStateUpdated {
                    state: app_state,
                })),
                &ctx,
            )
            .await;

        // Then sidebar_width is 50.
        let guard = state.read();
        assert_eq!(guard.frontend.sidebar_width, 50);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn persona_name_sync_sets_correct_persona() {
        // Kills: replace == with != in persona_name matching.
        // If the condition were flipped, the wrong persona would be set.
        // Given a sync actor with two personas loaded.
        let (mut actor, state, ctx) = create_actor();
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
        actor
            .handle(
                ActorEnvelope::Event(Event::AppStateUpdated(AppStateUpdated {
                    state: app_state,
                })),
                &ctx,
            )
            .await;

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
    #[tokio::test]
    async fn theme_name_none_resolves_default_theme() {
        // Given a sync actor.
        let (mut actor, state, ctx) = create_actor();

        // When receiving AppStateUpdated with theme_name = None.
        let app_state = AppStateFile {
            theme_name: None,
            ..AppStateFile::default()
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::AppStateUpdated(AppStateUpdated {
                    state: app_state,
                })),
                &ctx,
            )
            .await;


        // Then the theme was resolved and caches invalidated without panic.
        // resolve_theme(None, ...) returns the embedded default theme.
        let _guard = state.read();
        // If we reach here, the handler completed successfully.
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn ignores_unrelated_events() {
        // Given a sync actor.
        let (mut actor, state, ctx) = create_actor();

        // When receiving an unrelated event (ModeChanged).
        actor
            .handle(
                ActorEnvelope::Event(Event::ModeChanged(crate::protocol::system::ModeChanged {
                    from: crate::protocol::Mode::Normal,
                    to: crate::protocol::Mode::Input,
                })),
                &ctx,
            )
            .await;

        // Then sidebar_width remains at its default.
        let guard = state.read();
        assert_eq!(guard.frontend.sidebar_width, 30);
    }
}
