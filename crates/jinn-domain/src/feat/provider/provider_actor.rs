//! Provider actor - manages active provider, LLM factory, model cache, and picker entries.
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

use super::loader::{load_compaction_model_picker_items, load_provider_picker_items};
use crate::feat::provider::protocol::command::{
    LoadCompactionModelPickerEntries, LoadProviderPickerEntries,
};

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
        ctx.subscribe_command::<LoadCompactionModelPickerEntries>();
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
            Command::LoadCompactionModelPickerEntries(payload) => {
                self.handle_load_compaction_model_picker_entries(payload);
            }
            // Commands NOT subscribed to - these should not arrive.
            Command::SendMessage(..)
            | Command::PinChatEntry(..)
            | Command::UnpinChatEntry(..)
            | Command::EnqueueUserMessage(..)
            | Command::EnqueueResumeTurn(..)
            | Command::SetChatInputText(..)
            | Command::PushChatEntry(..)
            | Command::CancelStream(..)
            | Command::SendToLlmProvider(..)
            | Command::RefreshModels
            | Command::RescanPromptTemplates(..)
            | Command::ScanContextFiles(..)
            | Command::RegisterTools(..)
            | Command::ExecuteToolBatch(..)
            | Command::ExecuteTool(..)
            | Command::CancelToolBatch(..)
            | Command::ProceedWithShutdown(..)
            | Command::SessionLoadRequested(..)
            | Command::LoadSessionPickerEntries(..)
            | Command::ScanSkills(..)
            | Command::RescanPersonas(..)
            | Command::LoadPersonaPickerEntries(..)
            | Command::UpdatePreferences(..)
            | Command::SessionForkRequested(..)
            | Command::RunSessionSetup(..)
            | Command::RunSessionTeardown(..)
            | Command::CloseSession(..)
            | Command::ArchiveSession(..)
            | Command::PersistSession(..)
            | Command::SetSessionCwd(..)
            | Command::FinishSessionTeardown(..)
            | Command::FinishSessionSetup(..)
            | Command::CancelLifecycleCommand(..)
            | Command::MarkSessionInteracted(..)
            | Command::SubmitHistoryMutations(..)
            | Command::TriggerCompaction(..)
            | Command::Dynamic(..)
            | Command::ExecuteWebFetch(..)
            | Command::AttachPlugin(..)
            | Command::DetachPlugin(..)
            | Command::TogglePlugin(..)
            | Command::SubmitSteeringMessage(..) => {}
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

    /// LoadCompactionModelPickerEntries: load compaction model picker entries.
    fn handle_load_compaction_model_picker_entries(
        &self,
        _payload: &LoadCompactionModelPickerEntries,
    ) {
        let mut state = self.state.write();
        load_compaction_model_picker_items(&self.services, &mut state);
    }

    // --- Event handlers ---

    /// ModelsRefreshed: update model cache and reload provider picker entries.
    fn handle_models_refreshed(&self, event: &ModelsRefreshed) {
        let now = jiff::Timestamp::now();
        let mut cache = crate::feat::provider_infra::ModelCache {
            entries: event.results.clone(),
            last_updated_at: Some(now),
        };
        {
            let registry = self.services.provider_registry.read();
            merge_context_lengths_from_registry(&mut cache, &registry);
        }
        let models_dev = crate::feat::provider_infra::ModelsDevData::load(
            &self.services.paths.models_dev_user_path(),
            &self.services.paths.models_dev_system_path(),
        );
        merge_context_lengths_from_models_dev(&mut cache, &models_dev);
        // Merge remote models into the registry so create_factory() can find them.
        self.services.provider_registry.merge_cache(&cache);
        let mut state = self.state.write();
        state.provider.model_cache = Some(cache);
        // Also reload provider picker entries from updated model cache.
        load_provider_picker_items(&self.services, &mut state);
    }

    /// ModelCacheLoaded: restore model cache from disk and reload picker entries.
    fn handle_model_cache_loaded(&self, cache: &crate::feat::provider_infra::ModelCache) {
        let mut cache = cache.clone();
        {
            let registry = self.services.provider_registry.read();
            merge_context_lengths_from_registry(&mut cache, &registry);
        }
        let models_dev = crate::feat::provider_infra::ModelsDevData::load(
            &self.services.paths.models_dev_user_path(),
            &self.services.paths.models_dev_system_path(),
        );
        merge_context_lengths_from_models_dev(&mut cache, &models_dev);
        // Merge remote models into the registry so create_factory() can find them.
        self.services.provider_registry.merge_cache(&cache);
        let mut state = self.state.write();
        state.provider.model_cache = Some(cache);
        load_provider_picker_items(&self.services, &mut state);
    }
}

/// Merge `context_length` from the registry's resolved providers into the
/// model cache, filling in `None` slots where the API did not provide a value.
///
/// This is a conservative merge: API-provided values are never overwritten.
/// Only `None` entries are filled from the registry (which sources its value
/// from `providers.toml` manual overrides).
fn merge_context_lengths_from_registry(
    cache: &mut crate::feat::provider_infra::ModelCache,
    registry: &crate::feat::provider_infra::ProviderRegistry,
) {
    for provider in registry.providers() {
        let Some(registry_ctx) = provider.context_length else {
            continue;
        };
        let Some(models) = cache.entries.get_mut(&provider.name) else {
            continue;
        };
        for model in models.iter_mut() {
            if model.id == provider.model && model.context_length.is_none() {
                model.context_length = Some(registry_ctx);
            }
        }
    }
}

/// Merge `context_length` from the models.dev reference data into the
/// model cache, filling in `None` slots where neither the API nor
/// `providers.toml` provided a value.
///
/// This is the lowest-priority merge: only `None` entries are filled,
/// and existing values from the API or `providers.toml` are never overwritten.
fn merge_context_lengths_from_models_dev(
    cache: &mut crate::feat::provider_infra::ModelCache,
    models_dev: &crate::feat::provider_infra::ModelsDevData,
) {
    for models in cache.entries.values_mut() {
        for model in models.iter_mut() {
            if model.context_length.is_none()
                && let Some(ctx) = models_dev.get(&model.id)
            {
                model.context_length = Some(ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use std::sync::Arc;

    use crate::AppState;
    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::common::services::Services;
    use crate::common::state::State;
    use crate::feat::provider_infra::{ModelCache, ModelInfo, ProviderEntry, ProvidersConfig};
    use crate::protocol::{Command, Event};

    use super::{ModelsRefreshed, ProviderActor, ProviderActorDeps};
    use crate::feat::provider::protocol::command::LoadProviderPickerEntries;
    use crate::feat::provider::protocol::command::ProviderSwitch;
    use crate::feat::ui::picker_states::PickerExt;

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

    fn create_actor_with_config(
        config: ProvidersConfig,
    ) -> (
        ProviderActor,
        Services,
        Arc<RecordingSink>,
        ActorContext,
        State,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("provider", sink.clone() as Arc<dyn MessageSink>);
        let services = Services::new();
        let registry =
            crate::feat::provider_infra::ProviderRegistry::from_config(config).expect("registry");
        services.provider_registry.replace(registry);
        let state = State::new(AppState::default());
        let deps = ProviderActorDeps {
            services: services.clone(),
            state: state.clone(),
        };
        let actor = ProviderActor::activate(deps, &mut ctx);
        (actor, services, sink, ctx, state)
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

    // --- Context length merge tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_fills_context_length_from_registry_when_api_returns_none() {
        // Given a registry with zai provider that has context_length: Some(128_000).
        let config = ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "zai".to_owned(),
                backend: "zai".to_owned(),
                models: vec!["zai-1.5".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
                context_length: Some(128_000),
            }],
            aliases: vec![],
            default_provider: None,
        };
        let (mut actor, services, _sink, ctx, state) = create_actor_with_config(config);

        // When handling ModelsRefreshed with zai model that has context_length: None.
        let mut results = std::collections::HashMap::new();
        results.insert(
            "zai".to_owned(),
            vec![ModelInfo {
                id: "zai-1.5".to_owned(),
                context_length: None,
            }],
        );
        let event = ModelsRefreshed {
            session_id: state.read().session.active_session_id().clone(),
            results,
            errors: std::collections::HashMap::new(),
        };
        actor
            .handle(ActorEnvelope::Event(Event::ModelsRefreshed(event)), &ctx)
            .await;

        // Then the model cache has context_length from the registry.
        let s = state.read();
        let cache = s
            .provider
            .model_cache
            .as_ref()
            .expect("cache should be set");
        assert_eq!(cache.entries["zai"][0].context_length, Some(128_000));

        // And the model is registered in the provider registry.
        let resolved =
            services
                .provider_registry
                .get(&crate::feat::provider_infra::ProviderId::new(
                    "zai/zai-1.5".to_owned(),
                ));
        assert!(
            resolved.is_some(),
            "model should be in registry after ModelsRefreshed"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_preserves_api_context_length_when_both_sources_have_value() {
        // Given a registry with ollama provider that has context_length: Some(4096).
        let config = ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                models: vec!["llama3".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
                context_length: Some(4096),
            }],
            aliases: vec![],
            default_provider: None,
        };
        let (mut actor, _services, _sink, ctx, state) = create_actor_with_config(config);

        // When handling ModelsRefreshed where API returns context_length: Some(8192).
        let mut results = std::collections::HashMap::new();
        results.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: Some(8192),
            }],
        );
        let event = ModelsRefreshed {
            session_id: state.read().session.active_session_id().clone(),
            results,
            errors: std::collections::HashMap::new(),
        };
        actor
            .handle(ActorEnvelope::Event(Event::ModelsRefreshed(event)), &ctx)
            .await;

        // Then the API value wins (8192), not the registry value (4096).
        let s = state.read();
        let cache = s
            .provider
            .model_cache
            .as_ref()
            .expect("cache should be set");
        assert_eq!(cache.entries["ollama"][0].context_length, Some(8192));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_leaves_none_when_neither_source_has_context_length() {
        // Given a registry with provider that has context_length: None.
        let (mut actor, _services, _sink, ctx, state) = create_actor();

        // When handling ModelsRefreshed where API also returns context_length: None.
        let mut results = std::collections::HashMap::new();
        results.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: None,
            }],
        );
        let event = ModelsRefreshed {
            session_id: state.read().session.active_session_id().clone(),
            results,
            errors: std::collections::HashMap::new(),
        };
        actor
            .handle(ActorEnvelope::Event(Event::ModelsRefreshed(event)), &ctx)
            .await;

        // Then the cache entry stays None.
        let s = state.read();
        let cache = s
            .provider
            .model_cache
            .as_ref()
            .expect("cache should be set");
        assert_eq!(cache.entries["ollama"][0].context_length, None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_does_not_touch_provider_not_in_registry() {
        // Given a registry with only ollama provider.
        let (mut actor, _services, _sink, ctx, state) = create_actor();

        // When handling ModelsRefreshed with results for groq (not in registry).
        let mut results = std::collections::HashMap::new();
        results.insert(
            "groq".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: None,
            }],
        );
        let event = ModelsRefreshed {
            session_id: state.read().session.active_session_id().clone(),
            results,
            errors: std::collections::HashMap::new(),
        };
        actor
            .handle(ActorEnvelope::Event(Event::ModelsRefreshed(event)), &ctx)
            .await;

        // Then the cache entry is stored as-is, no panic.
        let s = state.read();
        let cache = s
            .provider
            .model_cache
            .as_ref()
            .expect("cache should be set");
        assert_eq!(cache.entries["groq"][0].context_length, None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn model_cache_loaded_fills_context_length_from_registry_when_cache_has_none() {
        // Given a registry with zai provider that has context_length: Some(128_000).
        let config = ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "zai".to_owned(),
                backend: "zai".to_owned(),
                models: vec!["zai-1.5".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
                context_length: Some(128_000),
            }],
            aliases: vec![],
            default_provider: None,
        };
        let (mut actor, services, _sink, ctx, state) = create_actor_with_config(config);

        // When handling ModelCacheLoaded with cache that has context_length: None.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "zai".to_owned(),
            vec![ModelInfo {
                id: "zai-1.5".to_owned(),
                context_length: None,
            }],
        );
        cache.last_updated_at = Some(jiff::Timestamp::now());

        let event = crate::feat::provider::protocol::event::ModelCacheLoaded {
            cache: cache.clone(),
        };
        actor
            .handle(ActorEnvelope::Event(Event::ModelCacheLoaded(event)), &ctx)
            .await;

        // Then the model cache in state has context_length from the registry.
        let s = state.read();
        let loaded = s
            .provider
            .model_cache
            .as_ref()
            .expect("cache should be set");
        assert_eq!(loaded.entries["zai"][0].context_length, Some(128_000));

        // And the model is registered in the provider registry.
        let resolved =
            services
                .provider_registry
                .get(&crate::feat::provider_infra::ProviderId::new(
                    "zai/zai-1.5".to_owned(),
                ));
        assert!(
            resolved.is_some(),
            "model should be in registry after ModelCacheLoaded"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn model_cache_loaded_preserves_cache_value_when_both_sources_have_value() {
        // Given a registry with ollama provider that has context_length: Some(4096).
        let config = ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                models: vec!["llama3".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
                context_length: Some(4096),
            }],
            aliases: vec![],
            default_provider: None,
        };
        let (mut actor, _services, _sink, ctx, state) = create_actor_with_config(config);

        // When handling ModelCacheLoaded with cache that has context_length: Some(8192).
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
        actor
            .handle(ActorEnvelope::Event(Event::ModelCacheLoaded(event)), &ctx)
            .await;

        // Then the cache value wins (8192), not the registry value (4096).
        let s = state.read();
        let loaded = s
            .provider
            .model_cache
            .as_ref()
            .expect("cache should be set");
        assert_eq!(loaded.entries["ollama"][0].context_length, Some(8192));
    }

    // --- models.dev merge tests ---

    #[rstest::rstest]
    fn merge_from_models_dev_fills_none() {
        // Given a cache with context_length: None and models.dev data.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "zai".to_owned(),
            vec![ModelInfo {
                id: "glm-5.1".to_owned(),
                context_length: None,
            }],
        );

        let mut models_dev = crate::feat::provider_infra::ModelsDevData::new();
        models_dev
            .context_lengths
            .insert("glm-5.1".to_owned(), 200_000);

        // When merging.
        super::merge_context_lengths_from_models_dev(&mut cache, &models_dev);

        // Then the model now has context_length from models.dev.
        assert_eq!(cache.entries["zai"][0].context_length, Some(200_000));
    }

    #[rstest::rstest]
    fn merge_from_models_dev_does_not_overwrite_existing() {
        // Given a cache with context_length: Some(100000) and models.dev has 200000.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "openai".to_owned(),
            vec![ModelInfo {
                id: "gpt-4o".to_owned(),
                context_length: Some(100_000),
            }],
        );

        let mut models_dev = crate::feat::provider_infra::ModelsDevData::new();
        models_dev
            .context_lengths
            .insert("gpt-4o".to_owned(), 200_000);

        // When merging.
        super::merge_context_lengths_from_models_dev(&mut cache, &models_dev);

        // Then the existing value is preserved.
        assert_eq!(cache.entries["openai"][0].context_length, Some(100_000));
    }

    #[rstest::rstest]
    fn merge_from_models_dev_leaves_none_when_not_in_data() {
        // Given a cache with an unknown model.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "local".to_owned(),
            vec![ModelInfo {
                id: "my-custom-llama".to_owned(),
                context_length: None,
            }],
        );

        let models_dev = crate::feat::provider_infra::ModelsDevData::new();

        // When merging with empty models.dev data.
        super::merge_context_lengths_from_models_dev(&mut cache, &models_dev);

        // Then it stays None.
        assert_eq!(cache.entries["local"][0].context_length, None);
    }

    #[rstest::rstest]
    fn merge_priority_is_api_then_config_then_models_dev() {
        // Given three models with different source scenarios.
        let mut cache = ModelCache::new();
        // Model A: API returned a value.
        cache.entries.insert(
            "provider-a".to_owned(),
            vec![ModelInfo {
                id: "model-a".to_owned(),
                context_length: Some(100_000),
            }],
        );
        // Model B: API returned None, config will fill it.
        cache.entries.insert(
            "provider-b".to_owned(),
            vec![ModelInfo {
                id: "model-b".to_owned(),
                context_length: None,
            }],
        );
        // Model C: API returned None, no config, models.dev should fill it.
        cache.entries.insert(
            "provider-c".to_owned(),
            vec![ModelInfo {
                id: "model-c".to_owned(),
                context_length: None,
            }],
        );
        // Model D: API returned None, no config, not in models.dev.
        cache.entries.insert(
            "provider-d".to_owned(),
            vec![ModelInfo {
                id: "model-d".to_owned(),
                context_length: None,
            }],
        );

        // Simulate config merge: set model-b to 64000.
        cache.entries.get_mut("provider-b").unwrap()[0].context_length = Some(64_000);

        let mut models_dev = crate::feat::provider_infra::ModelsDevData::new();
        models_dev
            .context_lengths
            .insert("model-a".to_owned(), 999_999);
        models_dev
            .context_lengths
            .insert("model-b".to_owned(), 999_999);
        models_dev
            .context_lengths
            .insert("model-c".to_owned(), 300_000);

        // When merging from models.dev.
        super::merge_context_lengths_from_models_dev(&mut cache, &models_dev);

        // Then: A keeps API value, B keeps config value, C gets models.dev, D stays None.
        assert_eq!(cache.entries["provider-a"][0].context_length, Some(100_000));
        assert_eq!(cache.entries["provider-b"][0].context_length, Some(64_000));
        assert_eq!(cache.entries["provider-c"][0].context_length, Some(300_000));
        assert_eq!(cache.entries["provider-d"][0].context_length, None);
    }

    #[rstest::rstest]
    fn merge_from_models_dev_handles_multiple_providers() {
        // Given two providers with models that have None.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "zai".to_owned(),
            vec![ModelInfo {
                id: "glm-5.1".to_owned(),
                context_length: None,
            }],
        );
        cache.entries.insert(
            "anthropic".to_owned(),
            vec![ModelInfo {
                id: "claude-sonnet-4-20250514".to_owned(),
                context_length: None,
            }],
        );

        let mut models_dev = crate::feat::provider_infra::ModelsDevData::new();
        models_dev
            .context_lengths
            .insert("glm-5.1".to_owned(), 200_000);
        models_dev
            .context_lengths
            .insert("claude-sonnet-4-20250514".to_owned(), 200_000);

        // When merging.
        super::merge_context_lengths_from_models_dev(&mut cache, &models_dev);

        // Then both providers get filled.
        assert_eq!(cache.entries["zai"][0].context_length, Some(200_000));
        assert_eq!(cache.entries["anthropic"][0].context_length, Some(200_000));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_dispatches_provider_switch_command() {
        // Kills: delete ActorEnvelope::Command(cmd) match arm in handle.
        // Also kills: replace handle_command with ().
        // Also kills: replace handle_provider_switch with ().
        // Given a provider actor.
        let (mut actor, _services, sink, ctx, state) = create_actor();
        let session_id = state.read().session.active_session_id().clone();

        // When sending a ProviderSwitch command.
        let cmd = Command::ProviderSwitch(ProviderSwitch {
            session_id: session_id.clone(),
            provider_id: "ollama/llama3".to_owned(),
        });
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then the session model is updated.
        let s = state.read();
        assert_eq!(s.session.active_session().profile().model, "ollama/llama3");

        // And a ProviderSwitched event is emitted.
        let events = sink.take_events();
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], Event::ProviderSwitched(e) if e.provider_name == "ollama/llama3")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_dispatches_load_provider_picker_entries_command() {
        // Kills: replace handle_load_provider_picker_entries with ().
        // Given a provider actor with a registry.
        let (mut actor, services, _sink, ctx, state) = create_actor();
        let registry = crate::feat::provider_infra::ProviderRegistry::from_config(sample_config())
            .expect("registry");
        services.provider_registry.replace(registry);

        // When sending LoadProviderPickerEntries.
        let cmd = Command::LoadProviderPickerEntries(LoadProviderPickerEntries);
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then the provider picker has entries.
        let s = state.read();
        let items = s.provider.provider_picker.items();
        assert!(
            !items.is_empty(),
            "picker should have entries after loading"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_dispatches_load_compaction_model_picker_entries_command() {
        // Kills: replace handle_load_compaction_model_picker_entries with ().
        // Given a provider actor with a registry.
        let (mut actor, services, _sink, ctx, state) = create_actor();
        let registry = crate::feat::provider_infra::ProviderRegistry::from_config(sample_config())
            .expect("registry");
        services.provider_registry.replace(registry);

        // When sending LoadCompactionModelPickerEntries.
        let cmd = Command::LoadCompactionModelPickerEntries(
            crate::feat::provider::protocol::command::LoadCompactionModelPickerEntries,
        );
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then the compaction model picker has entries (at least the sentinel).
        let s = state.read();
        let items = s.frontend.compaction_model_picker().items();
        assert!(
            !items.is_empty(),
            "compaction picker should have entries after loading"
        );
    }
}
