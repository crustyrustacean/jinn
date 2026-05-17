//! Provider initialization actor — loads static config, merges cache, resolves `last_model`.
//!
//! Subscribes to [`EnvironmentLoaded`](super::EnvironmentLoaded) emitted by the
//! env init actor. On receipt: builds the `ProviderRegistry` from the config,
//! replaces the empty startup registry, loads the model cache from disk, merges
//! cache entries into the registry, loads user preferences, and if `last_model`
//! is set, sends a `ProviderSwitch` command to apply it.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::provider::protocol::command::ProviderSwitch;
use crate::feat::provider_infra::{ModelCache, ProviderRegistry};
use crate::init::EnvironmentLoaded;
use crate::protocol::{Command, Event};

/// The provider initialization actor.
///
/// On `EnvironmentLoaded`: builds the registry from config, replaces the empty
/// registry in `ProviderRegistryService`, loads the model cache, merges into
/// registry, loads user preferences, and sends `ProviderSwitch` if `last_model`
/// is set.
pub struct ProviderInitActor {
    /// Shared services (registry, API keys, user preferences storage).
    services: Services,
    /// Shared application state (to read active session ID).
    state: State,
}

impl Actor for ProviderInitActor {
    type Message = NoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<EnvironmentLoaded>();
        ctx.set_description("Loads provider config, merges cache, resolves last_model");

        #[expect(
            clippy::expect_used,
            reason = "Services injection is required at activation"
        )]
        let services = ctx
            .take_data::<Services>()
            .expect("ProviderInitActor requires Services injection");
        let state = ctx
            .take_data::<State>()
            .expect("ProviderInitActor requires State injection");

        Self { services, state }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => {
                if let Event::EnvironmentLoaded(ref payload) = event {
                    self.on_environment_loaded(&payload.config, ctx);
                }
            }
            ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }
}

impl ProviderInitActor {
    /// Builds registry, merges cache, resolves `last_model`.
    fn on_environment_loaded(
        &self,
        config: &crate::feat::provider_infra::ProvidersConfig,
        ctx: &ActorContext,
    ) {
        // Build registry from config and replace the empty one.
        let registry = match ProviderRegistry::from_config(config.clone()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(err = ?e, "provider-init failed to build registry from config");
                return;
            }
        };
        self.services.provider_registry.replace(registry);

        // Check if no API keys were resolved. If so, push a guidance message.
        if self.services.api_keys.is_empty() {
            tracing::warn!("no API keys found, showing guidance message");
            self.state
                .write()
                .active_session_mut()
                .push_entry(crate::feat::session::no_api_keys_msg());
        }

        // Load model cache from disk and merge into registry.
        let cache_path = self.services.paths.cache_path();
        let cache = ModelCache::load(&cache_path).unwrap_or_else(|e| {
            tracing::warn!("provider-init failed to load model cache: {e:?}");
            None
        });
        if let Some(ref c) = cache {
            tracing::info!(providers = c.entries.len(), "loaded model cache");
            self.services.provider_registry.merge_cache(c);
        }

        // Load user preferences.
        let prefs = match self.services.user_preferences_storage.load() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("provider-init failed to load user preferences: {e:?}");
                return;
            }
        };

        // If last_model is set, send ProviderSwitch to apply it.
        if let Some(ref model) = prefs.last_model {
            let id = crate::feat::provider_infra::ProviderId::new(model.clone());
            let api_keys = self.services.api_keys.read();
            if self.services.provider_registry.is_available(&id, &api_keys) {
                tracing::info!(last_model = %model, "provider-init resolving last_model");
                if let Err(e) = ctx.send_command(Command::ProviderSwitch(ProviderSwitch {
                    session_id: self.state.read().session.active_session.clone(),
                    provider_id: model.clone(),
                })) {
                    tracing::warn!(err = ?e, "provider-init failed to send ProviderSwitch");
                }
            } else {
                tracing::warn!(last_model = %model, "provider-init: last_model not available, skipping");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::AppState;
    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::common::services::Services;
    use crate::common::state::State;
    use crate::feat::preferences_actor::user_preferences::UserPreferences;
    use crate::feat::provider_infra::ProviderEntry;
    use crate::init::EnvironmentLoaded;
    use crate::protocol::{Command, Event};

    use super::ProviderInitActor;

    /// Creates a test actor with Services defaults.
    fn create_actor() -> (
        ProviderInitActor,
        Services,
        Arc<RecordingSink>,
        ActorContext,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("provider-init", sink.clone() as Arc<dyn MessageSink>);

        let services = Services::new();
        let state = State::new(AppState::default());
        ctx.set_data(services.clone());
        ctx.set_data(state);
        let actor = ProviderInitActor::activate(&mut ctx);
        (actor, services, sink, ctx)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn sends_provider_switch_when_last_model_set() {
        // Given a provider init actor with preferences containing last_model.
        let (mut actor, services, sink, ctx) = create_actor();

        // Set up preferences with a last_model.
        services
            .user_preferences_storage
            .save(&UserPreferences {
                last_model: Some("sample/sample".to_owned()),
                last_strategy: None,
                tool_entry_max_lines: None,
                theme_name: None,
                persona_name: None,
                session_lifecycles: vec![],
            })
            .expect("save prefs");

        // Set up registry with a sample provider.
        let config = crate::feat::provider_infra::ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "sample".to_owned(),
                backend: "sample".to_owned(),
                models: vec!["sample".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
            }],
            aliases: vec![],
            default_provider: None,
        };

        // When processing EnvironmentLoaded.
        actor
            .handle(
                ActorEnvelope::Event(Event::EnvironmentLoaded(EnvironmentLoaded { config })),
                &ctx,
            )
            .await;

        // Then a ProviderSwitch command was sent.
        let commands = sink.commands();
        let found = commands.iter().any(|c| {
            matches!(c, Command::ProviderSwitch (payload) if payload.provider_id == "sample/sample")
        });
        assert!(found, "expected ProviderSwitch command for sample/sample");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn does_not_send_provider_switch_when_no_last_model() {
        // Given a provider init actor with no last_model in preferences.
        let (mut actor, _services, sink, ctx) = create_actor();

        let config = crate::feat::provider_infra::ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "sample".to_owned(),
                backend: "sample".to_owned(),
                models: vec!["sample".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
            }],
            aliases: vec![],
            default_provider: None,
        };

        // When processing EnvironmentLoaded.
        actor
            .handle(
                ActorEnvelope::Event(Event::EnvironmentLoaded(EnvironmentLoaded { config })),
                &ctx,
            )
            .await;

        // Then no ProviderSwitch command was sent.
        let commands = sink.commands();
        let found = commands
            .iter()
            .any(|c| matches!(c, Command::ProviderSwitch(..)));
        assert!(!found, "expected no ProviderSwitch command");
    }

    /// Creates a test actor with Services defaults, returning the shared state for assertions.
    fn create_actor_with_state() -> (
        ProviderInitActor,
        Services,
        Arc<RecordingSink>,
        ActorContext,
        State,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("provider-init", sink.clone() as Arc<dyn MessageSink>);

        let services = Services::new();
        let state = State::new(AppState::default());
        ctx.set_data(services.clone());
        ctx.set_data(state.clone());
        let actor = ProviderInitActor::activate(&mut ctx);
        (actor, services, sink, ctx, state)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn pushes_no_api_keys_msg_when_keys_empty() {
        // Given a provider init actor with no API keys.
        let (mut actor, _services, _sink, ctx, state) = create_actor_with_state();

        let config = crate::feat::provider_infra::ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "openrouter".to_owned(),
                backend: "openrouter".to_owned(),
                models: vec!["gpt-4".to_owned()],
                base_url: None,
                api_key_env: Some("OPENROUTER_API_KEY".to_owned()),
                requires_key: true,
                extra_body: None,
            }],
            aliases: vec![],
            default_provider: None,
        };

        // When processing EnvironmentLoaded with no API keys resolved.
        actor
            .handle(
                ActorEnvelope::Event(Event::EnvironmentLoaded(EnvironmentLoaded { config })),
                &ctx,
            )
            .await;

        // Then the active session has a no-api-keys info entry.
        let s = state.read();
        let text = s
            .active_session()
            .history()
            .last()
            .expect("at least one entry")
            .text();
        assert!(
            text.contains("No API keys found"),
            "should contain no-api-keys guidance, got: {text}"
        );
    }
}
