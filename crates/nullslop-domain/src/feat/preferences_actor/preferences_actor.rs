//! Preferences actor — persists user preferences to `nullslop.toml`.
//!
//! Subscribes to [`ProviderSwitched`] and [`PromptStrategySwitched`] events
//! emitted when the user selects a model or strategy from the pickers.
//! On each event, persists the corresponding preference to disk and syncs
//! the in-memory cache in `AppState.frontend.preferences`.
//!
//! # State ownership
//!
//! This actor writes `frontend.preferences` — the in-memory cache of
//! `nullslop.toml`. The file is the authoritative source; the cache
//! is a convenience for the sync IntentHandler.

use std::sync::Arc;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg, SystemMessage};
use crate::common::state::State;
use crate::feat::context::protocol::event::PromptStrategySwitched;
use crate::feat::preferences_actor::user_preferences::UserPreferences;
use crate::feat::preferences_actor::user_preferences_storage::UserPreferencesStorageService;
use crate::feat::provider::protocol::event::ProviderSwitched;
use crate::protocol::Event;

/// The preferences actor.
///
/// Subscribes to `ProviderSwitched` and `PromptStrategySwitched` events and
/// persists the selected model/strategy as `last_model`/`last_strategy`
/// in `nullslop.toml`.
pub struct PreferencesActor {
    /// User preferences storage service for reading/writing `nullslop.toml`.
    storage: UserPreferencesStorageService,
    /// Shared application state (to sync `frontend.preferences` cache).
    state: State,
}

impl Actor for PreferencesActor {
    type Message = NoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<ProviderSwitched>();
        ctx.subscribe_event::<PromptStrategySwitched>();
        ctx.set_description("Persists user preferences to nullslop.toml");

        #[expect(
            clippy::expect_used,
            reason = "Storage injection is required at activation"
        )]
        let storage = ctx
            .take_data::<UserPreferencesStorageService>()
            .expect("PreferencesActor requires UserPreferencesStorageService injection");
        #[expect(
            clippy::expect_used,
            reason = "State injection is required at activation"
        )]
        let state = ctx
            .take_data::<State>()
            .expect("PreferencesActor requires State injection");

        Self { storage, state }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => match event {
                Event::ProviderSwitched { ref payload } => {
                    self.handle_provider_switched(payload);
                }
                Event::PromptStrategySwitched { ref payload } => {
                    self.handle_prompt_strategy_switched(payload);
                }
                _ => {}
            },
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Command(_) | ActorEnvelope::Shutdown => {}
        }
    }
}

impl PreferencesActor {
    /// Persists the selected provider as `last_model` in `nullslop.toml`.
    fn handle_provider_switched(&self, payload: &ProviderSwitched) {
        let mut prefs = self.load_or_default();
        prefs.last_model = Some(payload.provider_name.clone());
        self.save_and_sync(&prefs);
    }

    /// Persists the selected strategy as `last_strategy` in `nullslop.toml`.
    fn handle_prompt_strategy_switched(&self, payload: &PromptStrategySwitched) {
        let mut prefs = self.load_or_default();
        prefs.last_strategy = Some(payload.strategy_id.as_str().to_owned());
        self.save_and_sync(&prefs);
    }

    /// Loads current preferences or returns defaults.
    fn load_or_default(&self) -> UserPreferences {
        match self.storage.load() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(err = ?e, "preferences-actor failed to load preferences");
                UserPreferences::default()
            }
        }
    }

    /// Saves preferences to disk and syncs the AppState cache.
    fn save_and_sync(&self, prefs: &UserPreferences) {
        if let Err(e) = self.storage.save(prefs) {
            tracing::warn!(err = ?e, "preferences-actor failed to save user preferences");
            return;
        }
        let mut state = self.state.write();
        state.frontend.preferences = prefs.clone();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::context::protocol::event::PromptStrategySwitched;
    use crate::feat::preferences_actor::user_preferences_storage::InMemoryUserPreferencesStorage;
    use crate::feat::preferences_actor::user_preferences_storage::UserPreferencesStorageService;
    use crate::feat::provider::protocol::event::ProviderSwitched;
    use crate::protocol::{Event, PromptStrategyId, SessionId};

    use super::PreferencesActor;

    /// Creates a test actor with in-memory storage.
    fn create_actor() -> (PreferencesActor, State, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("preferences-actor", sink.clone() as Arc<dyn MessageSink>);
        let storage =
            UserPreferencesStorageService::new(Arc::new(InMemoryUserPreferencesStorage::new()));
        let state = State::new(AppState::default());
        ctx.set_data(storage);
        ctx.set_data(state.clone());
        let actor = PreferencesActor::activate(&mut ctx);
        (actor, state, sink, ctx)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn provider_switched_saves_last_model() {
        // Given a preferences actor with in-memory storage.
        let (mut actor, _state, _sink, ctx) = create_actor();

        // When processing ProviderSwitched.
        actor
            .handle(
                ActorEnvelope::Event(Event::ProviderSwitched {
                    payload: ProviderSwitched {
                        provider_name: "ollama/llama3".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the storage contains the provider as last_model.
        let prefs = actor.storage.load().expect("load");
        assert_eq!(prefs.last_model.as_deref(), Some("ollama/llama3"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn provider_switched_overwrites_previous_model() {
        // Given a preferences actor with in-memory storage.
        let (mut actor, _state, _sink, ctx) = create_actor();

        // When processing ProviderSwitched twice.
        actor
            .handle(
                ActorEnvelope::Event(Event::ProviderSwitched {
                    payload: ProviderSwitched {
                        provider_name: "ollama/llama3".into(),
                    },
                }),
                &ctx,
            )
            .await;
        actor
            .handle(
                ActorEnvelope::Event(Event::ProviderSwitched {
                    payload: ProviderSwitched {
                        provider_name: "openrouter/gpt-4".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then only the latest model is persisted.
        let prefs = actor.storage.load().expect("load");
        assert_eq!(prefs.last_model.as_deref(), Some("openrouter/gpt-4"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn provider_switched_preserves_last_strategy() {
        // Given a preferences actor with a saved strategy.
        let (mut actor, _state, _sink, ctx) = create_actor();
        actor
            .handle(
                ActorEnvelope::Event(Event::PromptStrategySwitched {
                    payload: PromptStrategySwitched {
                        session_id: SessionId::new(),
                        strategy_id: PromptStrategyId::sliding_window(),
                    },
                }),
                &ctx,
            )
            .await;

        // When processing ProviderSwitched.
        actor
            .handle(
                ActorEnvelope::Event(Event::ProviderSwitched {
                    payload: ProviderSwitched {
                        provider_name: "ollama/llama3".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then last_strategy is preserved.
        let prefs = actor.storage.load().expect("load");
        assert_eq!(prefs.last_model.as_deref(), Some("ollama/llama3"));
        assert_eq!(prefs.last_strategy.as_deref(), Some("sliding_window"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strategy_switched_saves_last_strategy() {
        // Given a preferences actor.
        let (mut actor, _state, _sink, ctx) = create_actor();

        // When processing PromptStrategySwitched.
        actor
            .handle(
                ActorEnvelope::Event(Event::PromptStrategySwitched {
                    payload: PromptStrategySwitched {
                        session_id: SessionId::new(),
                        strategy_id: PromptStrategyId::sliding_window(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the storage contains the strategy as last_strategy.
        let prefs = actor.storage.load().expect("load");
        assert_eq!(prefs.last_strategy.as_deref(), Some("sliding_window"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strategy_switched_preserves_last_model() {
        // Given a preferences actor with a saved model.
        let (mut actor, _state, _sink, ctx) = create_actor();
        actor
            .handle(
                ActorEnvelope::Event(Event::ProviderSwitched {
                    payload: ProviderSwitched {
                        provider_name: "ollama/llama3".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // When processing PromptStrategySwitched.
        actor
            .handle(
                ActorEnvelope::Event(Event::PromptStrategySwitched {
                    payload: PromptStrategySwitched {
                        session_id: SessionId::new(),
                        strategy_id: PromptStrategyId::sliding_window(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then last_model is preserved.
        let prefs = actor.storage.load().expect("load");
        assert_eq!(prefs.last_model.as_deref(), Some("ollama/llama3"));
        assert_eq!(prefs.last_strategy.as_deref(), Some("sliding_window"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn provider_switched_syncs_app_state_cache() {
        // Given a preferences actor.
        let (mut actor, state, _sink, ctx) = create_actor();

        // When processing ProviderSwitched.
        actor
            .handle(
                ActorEnvelope::Event(Event::ProviderSwitched {
                    payload: ProviderSwitched {
                        provider_name: "ollama/llama3".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the AppState preferences cache is updated.
        let guard = state.read();
        assert_eq!(
            guard.frontend.preferences.last_model.as_deref(),
            Some("ollama/llama3")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strategy_switched_syncs_app_state_cache() {
        // Given a preferences actor.
        let (mut actor, state, _sink, ctx) = create_actor();

        // When processing PromptStrategySwitched.
        actor
            .handle(
                ActorEnvelope::Event(Event::PromptStrategySwitched {
                    payload: PromptStrategySwitched {
                        session_id: SessionId::new(),
                        strategy_id: PromptStrategyId::sliding_window(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the AppState preferences cache is updated.
        let guard = state.read();
        assert_eq!(
            guard.frontend.preferences.last_strategy.as_deref(),
            Some("sliding_window")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn ignores_unrelated_events() {
        // Given a preferences actor.
        let (mut actor, _state, _sink, ctx) = create_actor();

        // When processing an unrelated event (ModelsRefreshed).
        actor
            .handle(
                ActorEnvelope::Event(Event::ModelsRefreshed {
                    payload: crate::feat::provider::protocol::event::ModelsRefreshed {
                        session_id: crate::protocol::SessionId::new(),
                        results: std::collections::HashMap::new(),
                        errors: std::collections::HashMap::new(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then no preferences were saved (still defaults).
        let prefs = actor.storage.load().expect("load");
        assert!(prefs.last_model.is_none());
        assert!(prefs.last_strategy.is_none());
    }
}
