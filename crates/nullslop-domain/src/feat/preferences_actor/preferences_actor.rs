//! Preferences actor — persists user preferences to `nullslop.toml`.
//!
//! Subscribes to [`ProviderSwitched`] events emitted when the user selects
//! a model from the picker. On each event, writes the `last_model` preference
//! to disk via [`UserPreferencesStorageService`].
//!
//! # State ownership
//!
//! This actor owns no `AppState` fields. Its sole responsibility is writing
//! the preferences file. It does not read or write shared state.

use std::sync::Arc;

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::common::actor_host::{ActorSpawnResult, spawn_actor};
use crate::feat::preferences_actor::user_preferences::UserPreferences;
use crate::feat::preferences_actor::user_preferences_storage::UserPreferencesStorageService;
use crate::feat::provider::protocol::event::ProviderSwitched;
use crate::protocol::Event;

/// Direct message type (unused — the preferences actor only reacts to events).
pub enum PreferencesDirectMsg {}

/// The preferences actor.
///
/// Subscribes to `ProviderSwitched` events and persists the selected model
/// as `last_model` in `nullslop.toml`.
pub struct PreferencesActor {
    /// User preferences storage service for reading/writing `nullslop.toml`.
    storage: UserPreferencesStorageService,
}

impl Actor for PreferencesActor {
    type Message = PreferencesDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<ProviderSwitched>();
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
            ActorEnvelope::Event(event) => {
                if let Event::ProviderSwitched { ref payload } = event {
                    self.handle_provider_switched(payload);
                }
            }
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Command(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl PreferencesActor {
    /// Persists the selected provider as `last_model` in `nullslop.toml`.
    fn handle_provider_switched(&self, payload: &ProviderSwitched) {
        let prefs = UserPreferences {
            last_model: Some(payload.provider_name.clone()),
        };

        if let Err(e) = self.storage.save(&prefs) {
            tracing::warn!(err = ?e, "preferences-actor failed to save user preferences");
        }
    }
}

/// Spawns the preferences actor on the given tokio runtime.
///
/// Creates the actor's channel, context, and run loop. Injects
/// [`UserPreferencesStorageService`]. Returns the `ActorRef` for
/// sending direct messages and the `ActorSpawnResult` containing
/// the routing entry and join handle.
pub fn spawn_preferences_actor(
    storage: UserPreferencesStorageService,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<PreferencesDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<PreferencesDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("preferences", sink);
    ctx.set_description("Persists user preferences to nullslop.toml");
    ctx.set_data(storage);
    let actor = PreferencesActor::activate(&mut ctx);
    let result = spawn_actor("preferences", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::feat::preferences_actor::user_preferences_storage::InMemoryUserPreferencesStorage;
    use crate::feat::preferences_actor::user_preferences_storage::UserPreferencesStorageService;
    use crate::feat::provider::protocol::event::ProviderSwitched;
    use crate::protocol::Event;

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
    async fn provider_switched_saves_last_model() {
        // Given a preferences actor with in-memory storage.
        let (mut actor, _sink, ctx) = create_actor();

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
        let (mut actor, _sink, ctx) = create_actor();

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
    async fn ignores_unrelated_events() {
        // Given a preferences actor.
        let (mut actor, _sink, ctx) = create_actor();

        // When processing an unrelated event (ModelsRefreshed).
        actor
            .handle(
                ActorEnvelope::Event(Event::ModelsRefreshed {
                    payload: crate::feat::provider::protocol::event::ModelsRefreshed {
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
    }
}
