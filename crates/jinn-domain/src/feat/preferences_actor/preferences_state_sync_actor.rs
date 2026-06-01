//! Preferences state sync actor - keeps `AppState.frontend.preferences` in sync.
//!
//! Subscribes to [`PreferencesUpdated`] events emitted by [`PreferencesActor`].
//! On each event, replaces `state.frontend.preferences` with the full payload.
//! This is the ONLY actor that writes to `frontend.preferences`.

use std::path::PathBuf;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::theme;
use crate::protocol::Event;

/// Keeps `AppState.frontend.preferences` in sync with persisted preferences.
///
/// Subscribes to `PreferencesUpdated` events and writes the full preferences
/// to the shared state. This is the single writer for `frontend.preferences`.
pub struct PreferencesStateSyncActor {
    /// Shared application state.
    state: State,
    /// Path to the user themes directory.
    themes_dir: PathBuf,
    /// Path to the system themes directory.
    system_themes_dir: PathBuf,
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
            themes_dir: deps.services.paths.themes_dir(),
            system_themes_dir: deps.services.paths.system_themes_dir(),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::PreferencesUpdated(ref payload)) => {
                let mut state = self.state.write();
                state.frontend.preferences = payload.preferences.clone();
                // Reload theme when theme_name changes in preferences.
                match theme::resolve_theme(
                    payload.preferences.theme_name.as_deref(),
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

                // Sync active_persona when persona_name changes in preferences.
                if let Some(ref persona_name) = payload.preferences.persona_name {
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

                state.frontend.sidebar_width = payload.preferences.sidebar_width.unwrap_or(30);
            }
            ActorEnvelope::Command(_) | ActorEnvelope::Event(_) | ActorEnvelope::System(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use std::sync::Arc;

    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::common::app_paths::AppPaths;
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;
    use crate::feat::preferences_actor::user_preferences::UserPreferences;
    use crate::feat::preferences_actor::user_preferences::{
        AutoPruneConfig, CompactionConfig, ContextSlidingWindowConfig, CwdSelectorConfig,
        MinimapConfig, OpenrouterWebSearchConfig, WebFetchConfig,
    };
    use crate::protocol::Event;

    use super::{PreferencesStateSyncActor, PreferencesStateSyncActorDeps};
    use crate::common::services::Services;
    use crate::feat::preferences_actor::RequestRetryConfig;

    /// Creates a test actor with shared state.
    fn create_actor() -> (PreferencesStateSyncActor, State, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("preferences-sync", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        let deps = PreferencesStateSyncActorDeps {
            services: Services::new(),
            state: state.clone(),
        };

        let actor = PreferencesStateSyncActor::activate(deps, &mut ctx);
        (actor, state, ctx)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn preferences_updated_syncs_to_app_state() {
        // Given a sync actor.
        let (mut actor, state, ctx) = create_actor();

        // When receiving PreferencesUpdated with a model and strategy.
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: Some("sliding_window".to_owned()),
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::PreferencesUpdated(PreferencesUpdated {
                    preferences: prefs,
                })),
                &ctx,
            )
            .await;

        // Then AppState.frontend.preferences is updated.
        let guard = state.read();
        assert_eq!(
            guard.frontend.preferences.last_model.as_deref(),
            Some("ollama/llama3")
        );
        assert_eq!(
            guard.frontend.preferences.last_strategy.as_deref(),
            Some("sliding_window")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn second_event_overwrites_first() {
        // Given a sync actor with one update already applied.
        let (mut actor, state, ctx) = create_actor();
        let first = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::PreferencesUpdated(PreferencesUpdated {
                    preferences: first,
                })),
                &ctx,
            )
            .await;

        // When receiving a second PreferencesUpdated.
        let second = UserPreferences {
            last_model: Some("openrouter/gpt-4".to_owned()),
            last_strategy: Some("sliding_window".to_owned()),
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: None,
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::PreferencesUpdated(PreferencesUpdated {
                    preferences: second,
                })),
                &ctx,
            )
            .await;

        // Then AppState reflects the second update.
        let guard = state.read();
        assert_eq!(
            guard.frontend.preferences.last_model.as_deref(),
            Some("openrouter/gpt-4")
        );
        assert_eq!(
            guard.frontend.preferences.last_strategy.as_deref(),
            Some("sliding_window")
        );
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

        // Then preferences remain at defaults.
        let guard = state.read();
        assert!(guard.frontend.preferences.last_model.is_none());
        assert!(guard.frontend.preferences.last_strategy.is_none());
    }

    // --- S-Tier: Kill mutant for persona_name == condition ---

    #[rstest::rstest]
    #[tokio::test]
    async fn persona_name_sync_sets_correct_persona() {
        // Kills: replace == with != in persona_name matching.
        // If the condition were flipped, the wrong persona would be set.
        use crate::feat::persona::Persona;

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

        // When receiving PreferencesUpdated with persona_name = "writer".
        let prefs = UserPreferences {
            last_model: None,
            last_strategy: None,
            tool_entry_max_lines: None,
            min_collapse_count: None,
            theme_name: None,
            persona_name: Some("writer".to_owned()),
            session_lifecycles: vec![],
            sidebar_width: None,
            max_tool_output_lines: None,
            max_tool_output_bytes: None,
            compaction: CompactionConfig::default(),
            context_sliding_window: ContextSlidingWindowConfig::default(),
            request_retry: RequestRetryConfig::default(),
            web_fetch: WebFetchConfig::default(),
            openrouter_web_search: OpenrouterWebSearchConfig::default(),
            cwd_selector: CwdSelectorConfig::default(),
            minimap: MinimapConfig::default(),
            auto_prune: AutoPruneConfig::default(),
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::PreferencesUpdated(PreferencesUpdated {
                    preferences: prefs,
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
}
