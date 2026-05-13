//! Preferences actor — persists user preferences to `nullslop.toml`.
//!
//! Subscribes to [`UpdatePreferences`] commands carrying batches of
//! [`PreferenceUpdate`] diffs. On each command, loads current preferences,
//! applies all diffs, saves to disk, and emits a [`PreferencesUpdated`]
//! event with the full result.
//!
//! # State ownership
//!
//! This actor does **not** write `AppState.frontend.preferences` directly.
//! That is the exclusive responsibility of `PreferencesStateSyncActor`,
//! which subscribes to the `PreferencesUpdated` events emitted here.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg, SystemMessage};
use crate::feat::preferences_actor::protocol::command::UpdatePreferences;
use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;
use crate::feat::preferences_actor::user_preferences::UserPreferences;
use crate::feat::preferences_actor::user_preferences_storage::UserPreferencesStorageService;
use crate::protocol::{Command, Event};

/// The preferences actor.
///
/// Subscribes to `UpdatePreferences` commands and persists preference
/// diffs to `nullslop.toml`, then emits `PreferencesUpdated` so
/// downstream actors can sync their caches.
pub struct PreferencesActor {
    /// User preferences storage service for reading/writing `nullslop.toml`.
    storage: UserPreferencesStorageService,
}

impl Actor for PreferencesActor {
    type Message = NoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<UpdatePreferences>();
        ctx.set_description("Persists user preferences to nullslop.toml");

        #[expect(
            clippy::expect_used,
            reason = "Storage injection is required at activation"
        )]
        let storage = ctx
            .take_data::<UserPreferencesStorageService>()
            .expect("PreferencesActor requires UserPreferencesStorageService injection");

        Self { storage }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(Command::UpdatePreferences { ref payload }) => {
                self.handle_update_preferences(payload, ctx);
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Event(_) | ActorEnvelope::Command(_) | ActorEnvelope::Shutdown => {}
        }
    }
}

impl PreferencesActor {
    /// Processes a batch of preference diffs: load, apply, save, emit.
    fn handle_update_preferences(&self, payload: &UpdatePreferences, ctx: &ActorContext) {
        let mut prefs = self.load_or_default();
        for update in &payload.updates {
            update.apply(&mut prefs);
        }
        if let Err(e) = self.storage.save(&prefs) {
            tracing::warn!(err = ?e, "preferences-actor failed to save user preferences");
            return;
        }
        if let Err(e) = ctx.send_event(Event::PreferencesUpdated {
            payload: PreferencesUpdated { preferences: prefs },
        }) {
            tracing::warn!(err = ?e, "preferences-actor failed to emit PreferencesUpdated");
        }
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
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
    use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;
    use crate::feat::preferences_actor::user_preferences_storage::InMemoryUserPreferencesStorage;
    use crate::feat::preferences_actor::user_preferences_storage::UserPreferencesStorageService;
    use crate::protocol::{Command, Event};

    use super::PreferencesActor;

    /// Creates a test actor with in-memory storage.
    fn create_actor() -> (PreferencesActor, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("preferences-actor", sink.clone() as Arc<dyn MessageSink>);
        let storage =
            UserPreferencesStorageService::new(Arc::new(InMemoryUserPreferencesStorage::new()));
        ctx.set_data(storage);
        let actor = PreferencesActor::activate(&mut ctx);
        (actor, sink, ctx)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn set_last_model_saves_to_storage() {
        // Given a preferences actor with in-memory storage.
        let (mut actor, _sink, ctx) = create_actor();

        // When sending UpdatePreferences with SetLastModel.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastModel(Some("ollama/llama3".into()))],
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
    async fn set_last_model_overwrites_previous() {
        // Given a preferences actor with a saved model.
        let (mut actor, _sink, ctx) = create_actor();
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastModel(Some("ollama/llama3".into()))],
                    },
                }),
                &ctx,
            )
            .await;

        // When sending a second UpdatePreferences with a different model.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastModel(Some(
                            "openrouter/gpt-4".into(),
                        ))],
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
    async fn set_last_model_preserves_last_strategy() {
        // Given a preferences actor with a saved strategy.
        let (mut actor, _sink, ctx) = create_actor();
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastStrategy(Some(
                            "sliding_window".into(),
                        ))],
                    },
                }),
                &ctx,
            )
            .await;

        // When sending UpdatePreferences with SetLastModel.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastModel(Some("ollama/llama3".into()))],
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
    async fn set_last_strategy_saves_to_storage() {
        // Given a preferences actor.
        let (mut actor, _sink, ctx) = create_actor();

        // When sending UpdatePreferences with SetLastStrategy.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastStrategy(Some(
                            "sliding_window".into(),
                        ))],
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
    async fn set_last_strategy_preserves_last_model() {
        // Given a preferences actor with a saved model.
        let (mut actor, _sink, ctx) = create_actor();
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastModel(Some("ollama/llama3".into()))],
                    },
                }),
                &ctx,
            )
            .await;

        // When sending UpdatePreferences with SetLastStrategy.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastStrategy(Some(
                            "sliding_window".into(),
                        ))],
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
    async fn batch_diffs_apply_all_at_once() {
        // Given a preferences actor.
        let (mut actor, _sink, ctx) = create_actor();

        // When sending UpdatePreferences with both diffs in one batch.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![
                            PreferenceUpdate::SetLastModel(Some("ollama/llama3".into())),
                            PreferenceUpdate::SetLastStrategy(Some("sliding_window".into())),
                        ],
                    },
                }),
                &ctx,
            )
            .await;

        // Then both fields are persisted.
        let prefs = actor.storage.load().expect("load");
        assert_eq!(prefs.last_model.as_deref(), Some("ollama/llama3"));
        assert_eq!(prefs.last_strategy.as_deref(), Some("sliding_window"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn emits_preferences_updated_event() {
        // Given a preferences actor.
        let (mut actor, sink, ctx) = create_actor();

        // When sending UpdatePreferences.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastModel(Some("ollama/llama3".into()))],
                    },
                }),
                &ctx,
            )
            .await;

        // Then a PreferencesUpdated event was emitted with the full preferences.
        let events = sink.events();
        let found = events.iter().any(|e| {
            matches!(
                e,
                Event::PreferencesUpdated {
                    payload: PreferencesUpdated {
                        preferences
                    }
                } if preferences.last_model.as_deref() == Some("ollama/llama3")
            )
        });
        assert!(
            found,
            "expected PreferencesUpdated event with last_model=ollama/llama3"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn empty_diffs_does_not_change_storage() {
        // Given a preferences actor with a saved model.
        let (mut actor, _sink, ctx) = create_actor();
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences {
                        updates: vec![PreferenceUpdate::SetLastModel(Some("ollama/llama3".into()))],
                    },
                }),
                &ctx,
            )
            .await;

        // When sending UpdatePreferences with empty diffs.
        actor
            .handle(
                ActorEnvelope::Command(Command::UpdatePreferences {
                    payload: UpdatePreferences { updates: vec![] },
                }),
                &ctx,
            )
            .await;

        // Then the existing preferences are preserved.
        let prefs = actor.storage.load().expect("load");
        assert_eq!(prefs.last_model.as_deref(), Some("ollama/llama3"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn ignores_unrelated_commands() {
        // Given a preferences actor.
        let (mut actor, _sink, ctx) = create_actor();

        // When sending an unrelated command (RefreshModels).
        actor
            .handle(ActorEnvelope::Command(Command::RefreshModels), &ctx)
            .await;

        // Then no preferences were saved (still defaults).
        let prefs = actor.storage.load().expect("load");
        assert!(prefs.last_model.is_none());
        assert!(prefs.last_strategy.is_none());
    }
}
