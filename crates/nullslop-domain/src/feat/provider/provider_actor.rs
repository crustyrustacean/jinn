//! Provider actor — manages active provider, LLM factory, model cache, and picker entries.
//!
//! Subscribes to provider-related commands and events, mutates the corresponding
//! [`AppState`](crate::common::app_state::AppState) fields, and emits events for
//! other actors to react to.
//!
//! # State ownership
//!
//! This actor is the **sole writer** of the following `AppState` fields:
//! - `active_provider`
//! - `model_cache`
//! - `provider_picker` entries (via the loader)
//!
//! # Lock discipline
//!
//! All handlers follow the same pattern: acquire state lock → mutate → release →
//! then emit. Never hold the lock during emission.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::provider::protocol::command::ProviderSwitch;
use crate::feat::provider::protocol::event::{ModelCacheLoaded, ModelsRefreshed, ProviderSwitched};
use crate::protocol::{Command, Event};

use super::loader::load_provider_picker_items;
use crate::feat::provider::protocol::command::LoadProviderPickerEntries;

/// The provider actor.
///
/// Subscribes to provider-related commands, mutates [`State`], and emits events
/// via the [`ActorContext`] message sink.
pub struct ProviderActor {
    /// Shared application state.
    state: State,
    /// Runtime services (provider registry, API keys, LLM service factory).
    services: Services,
}

/// Dependencies for [`ProviderActor`].
pub struct ProviderActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
}

impl Actor for ProviderActor {
    type Message = NoDirectMsg;
    type Deps = ProviderActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<ProviderSwitch>();
        ctx.subscribe_command::<LoadProviderPickerEntries>();
        ctx.subscribe_event::<ModelsRefreshed>();
        ctx.subscribe_event::<ModelCacheLoaded>();

        ctx.set_description("Manages provider selection, LLM factory, and model cache");

        Self {
            state: deps.state,
            services: deps.services,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx),
            ActorEnvelope::Event(Event::ModelsRefreshed(ref payload)) => {
                self.handle_models_refreshed(payload);
            }
            ActorEnvelope::Event(Event::ModelCacheLoaded(ref payload)) => {
                self.handle_model_cache_loaded(&payload.cache);
            }
            _ => {}
        }
    }
}

impl ProviderActor {
    /// Dispatches a command to the appropriate handler.
    fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::ProviderSwitch(payload) => {
                self.handle_provider_switch(payload, ctx);
            }
            Command::LoadProviderPickerEntries(payload) => {
                self.handle_load_provider_picker_entries(payload);
            }
            // Commands NOT subscribed to — these should not arrive.
            Command::SendMessage(..)
            | Command::PinChatEntry(..)
            | Command::UnpinChatEntry(..)
            | Command::EnqueueUserMessage(..)
            | Command::SetChatInputText(..)
            | Command::PushChatEntry(..)
            | Command::CancelStream(..)
            | Command::AssemblePrompt(..)
            | Command::SendToLlmProvider(..)
            | Command::RefreshModels
            | Command::RescanPromptTemplates
            | Command::RegisterTools(..)
            | Command::ExecuteToolBatch(..)
            | Command::ExecuteTool(..)
            | Command::CancelToolBatch(..)
            | Command::ProceedWithShutdown(..)
            | Command::SessionLoadCompleted(..)
            | Command::SessionLoadRequested(..)
            | Command::LoadSessionPickerEntries(..)
            | Command::ScanSkills
            | Command::RescanPersonas(..)
            | Command::LoadPersonaPickerEntries(..)
            | Command::UpdatePreferences(..)
            | Command::SessionForkRequested(..)
            | Command::RunSessionSetup(..)
            | Command::RunSessionTeardown(..)
            | Command::CompactContext(..)
            | Command::BeginCompaction(..)
            | Command::CancelCompaction(..)
            | Command::EndCompaction(..)
            | Command::CloseSession(..)
            | Command::ArchiveSession(..)
            | Command::PersistSession(..) => {}
        }
    }

    // --- Command handlers ---

    /// ProviderSwitch: update session profile and emit ProviderSwitched event.
    fn handle_provider_switch(&self, payload: &ProviderSwitch, ctx: &ActorContext) {
        {
            let mut state = self.state.write();
            state
                .session_mut_or_create(&payload.session_id)
                .set_model(payload.provider_id.clone());
        }

        if let Err(e) = ctx.send_event(Event::ProviderSwitched(ProviderSwitched {
            session_id: payload.session_id.clone(),
            provider_name: payload.provider_id.clone(),
        })) {
            tracing::warn!(err = ?e, "provider-actor failed to emit ProviderSwitched");
        }
    }

    /// LoadProviderPickerEntries: load provider picker entries.
    fn handle_load_provider_picker_entries(&self, _payload: &LoadProviderPickerEntries) {
        let mut state = self.state.write();
        load_provider_picker_items(&self.services, &mut state);
    }

    // --- Event handlers ---

    /// ModelsRefreshed: update model cache and reload provider picker entries.
    fn handle_models_refreshed(&self, event: &ModelsRefreshed) {
        let now = jiff::Timestamp::now();
        let mut state = self.state.write();
        state.provider.model_cache = Some(crate::feat::provider_infra::ModelCache {
            entries: event.results.clone(),
            last_updated_at: Some(now),
        });
        // Also reload provider picker entries from updated model cache.
        load_provider_picker_items(&self.services, &mut state);
    }

    /// ModelCacheLoaded: restore model cache from disk and reload picker entries.
    fn handle_model_cache_loaded(&self, cache: &crate::feat::provider_infra::ModelCache) {
        let mut state = self.state.write();
        state.provider.model_cache = Some(cache.clone());
        load_provider_picker_items(&self.services, &mut state);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use std::sync::Arc;

    use crate::AppState;
    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::common::services::Services;
    use crate::common::state::State;
    use crate::feat::provider_infra::{ModelCache, ModelInfo, ProviderEntry, ProvidersConfig};
    use crate::protocol::Event;

    use super::{ProviderActor, ProviderActorDeps};

    fn create_actor() -> (
        ProviderActor,
        Services,
        Arc<RecordingSink>,
        ActorContext,
        State,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("provider", sink.clone() as Arc<dyn MessageSink>);

        let services = Services::new();
        let state = State::new(AppState::default());
        let deps = ProviderActorDeps {
            services: services.clone(),
            state: state.clone(),
        };
        let actor = ProviderActor::activate(deps, &mut ctx);
        (actor, services, sink, ctx, state)
    }

    fn sample_config() -> ProvidersConfig {
        ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                models: vec!["llama3".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
                context_length: None,
            }],
            aliases: vec![],
            default_provider: None,
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn model_cache_loaded_sets_model_cache_in_state() {
        // Given a provider actor and a registry with a provider.
        let (mut actor, services, _sink, ctx, state) = create_actor();
        let registry = crate::feat::provider_infra::ProviderRegistry::from_config(sample_config())
            .expect("registry");
        services.provider_registry.replace(registry);

        let mut cache = ModelCache::new();
        cache.entries.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: Some(8192),
            }],
        );
        cache.last_updated_at = Some(jiff::Timestamp::now());

        let event = crate::feat::provider::protocol::event::ModelCacheLoaded {
            cache: cache.clone(),
        };

        // When handling ModelCacheLoaded.
        actor
            .handle(ActorEnvelope::Event(Event::ModelCacheLoaded(event)), &ctx)
            .await;

        // Then the model cache is set in state.
        let s = state.read();
        assert!(s.provider.model_cache.is_some());
        let loaded = s.provider.model_cache.as_ref().unwrap();
        assert_eq!(loaded.entries["ollama"].len(), 1);
        assert_eq!(loaded.entries["ollama"][0].id, "llama3");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn model_cache_loaded_preserves_timestamp() {
        // Given a provider actor with a cache that has a timestamp.
        let (mut actor, services, _sink, ctx, state) = create_actor();
        let registry = crate::feat::provider_infra::ProviderRegistry::from_config(sample_config())
            .expect("registry");
        services.provider_registry.replace(registry);

        let ts = jiff::Timestamp::now();
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: None,
            }],
        );
        cache.last_updated_at = Some(ts);

        let event = crate::feat::provider::protocol::event::ModelCacheLoaded {
            cache: cache.clone(),
        };

        // When handling ModelCacheLoaded.
        actor
            .handle(ActorEnvelope::Event(Event::ModelCacheLoaded(event)), &ctx)
            .await;

        // Then the timestamp is preserved in state.
        let s = state.read();
        let loaded = s.provider.model_cache.as_ref().unwrap();
        assert!(loaded.last_updated_at.is_some());
    }
}
