//! Provider actor - manages active provider, LLM factory, model cache, and picker entries.
//!
//! Subscribes to provider-related commands and events, mutates the corresponding
//! [`AppState`](crate::common::app_state::AppState) fields, and emits events for
//! other actors to react to.
//!
//! # State ownership
//!
//! This actor **owns** the following `AppState` fields:
//! - `active_provider`
//! - `model_cache`
//! - `provider_picker` entries (via the loader)
//!
//! # Lock discipline
//!
//! All handlers follow the same pattern: acquire state lock → mutate → release →
//! then emit. Never hold the lock during emission.

use std::convert::Infallible;

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::state::State;
use crate::common::tcaps::provider::{FrontendProviderPickerWrite, ModelCacheWrite, ProviderCap};
use crate::common::tcaps::session::SessionCap;
use crate::feat::provider::protocol::command::{
    LoadCompactionModelPickerEntries, LoadEndpointPickerEntries, LoadProviderPickerEntries,
    LoadReasoningEffortPickerEntries, ProviderSwitch, RefreshEndpointPickerEntries,
};
use crate::feat::provider::protocol::event::{ModelCacheLoaded, ModelsRefreshed, ProviderSwitched};

use super::loader::{
    build_endpoint_entries, fetch_endpoints, load_compaction_model_picker_items,
    load_provider_picker_items, load_reasoning_effort_picker_items, resolve_openrouter_target,
    set_endpoint_picker_items, unavailable_endpoint_entries,
};
use crate::feat::endpoint::picker_entry::EndpointEntry;
use kameo::Actor;
use kameo::actor::ActorRef;
use kameo::message::{Context as MsgContext, Message};

/// The provider actor.
///
/// Subscribes to provider-related commands, mutates [`State`], and emits events
/// via the bus.
pub struct ProviderActor {
    /// Shared application state.
    state: State,
    /// Runtime services (provider registry, API keys, LLM service factory).
    deps: ActorDeps,
    /// Authority to write [`ProviderState`] via [`State::with_provider`].
    cap: ProviderCap,
    /// Authority to write the session model ([`SessionCap`]) — used by
    /// `handle_provider_switch` to set the session's active model.
    session_cap: SessionCap,
    /// In-memory, per-model cache of OpenRouter routing endpoints for the
    /// application's lifetime (not persisted to disk). Keyed by resolved model
    /// id; value is the parsed upstream list plus the fetch timestamp. The
    /// picker serves from this on open and re-fetches on-demand via `<c-r>`.
    endpoints_cache:
        std::collections::HashMap<String, (Vec<jinn_provider::EndpointInfo>, jiff::Timestamp)>,
}

/// Dependencies for [`ProviderActor`].
#[derive(Clone)]
pub struct ProviderActorDeps {
    /// Shared application state.
    pub state: State,
    /// Actor dependencies (services including bus).
    pub deps: ActorDeps,
    pub cap: ProviderCap,
    /// Authority to write the session model.
    pub session_cap: SessionCap,
}

impl Actor for ProviderActor {
    type Args = ProviderActorDeps;
    type Error = Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let bus = &args.deps.services.bus;
        bus.subscribe::<ProviderSwitch, _>(&actor_ref).await;
        bus.subscribe::<LoadProviderPickerEntries, _>(&actor_ref)
            .await;
        bus.subscribe::<LoadCompactionModelPickerEntries, _>(&actor_ref)
            .await;
        bus.subscribe::<LoadReasoningEffortPickerEntries, _>(&actor_ref)
            .await;
        bus.subscribe::<LoadEndpointPickerEntries, _>(&actor_ref)
            .await;
        bus.subscribe::<RefreshEndpointPickerEntries, _>(&actor_ref)
            .await;
        bus.subscribe::<ModelsRefreshed, _>(&actor_ref).await;
        bus.subscribe::<ModelCacheLoaded, _>(&actor_ref).await;

        Ok(Self {
            state: args.state,
            deps: args.deps,
            cap: args.cap,
            session_cap: args.session_cap,
            endpoints_cache: std::collections::HashMap::new(),
        })
    }
}

impl Message<ProviderSwitch> for ProviderActor {
    type Reply = ();

    async fn handle(&mut self, msg: ProviderSwitch, _ctx: &mut MsgContext<Self, Self::Reply>) {
        self.handle_provider_switch(&msg);
        self.publish(ProviderSwitched {
            session_id: msg.session_id.clone(),
            provider_name: msg.provider_id.to_string(),
        })
        .await;
    }
}

impl Message<LoadProviderPickerEntries> for ProviderActor {
    type Reply = ();

    async fn handle(
        &mut self,
        _msg: LoadProviderPickerEntries,
        _ctx: &mut MsgContext<Self, Self::Reply>,
    ) {
        self.state.with_provider(&self.cap, |view| {
            load_provider_picker_items(&self.deps.services, view);
        });
    }
}

impl Message<LoadCompactionModelPickerEntries> for ProviderActor {
    type Reply = ();

    async fn handle(
        &mut self,
        _msg: LoadCompactionModelPickerEntries,
        _ctx: &mut MsgContext<Self, Self::Reply>,
    ) {
        self.state.with_provider(&self.cap, |view| {
            load_compaction_model_picker_items(&self.deps.services, view);
        });
    }
}

impl Message<LoadReasoningEffortPickerEntries> for ProviderActor {
    type Reply = ();

    async fn handle(
        &mut self,
        _msg: LoadReasoningEffortPickerEntries,
        _ctx: &mut MsgContext<Self, Self::Reply>,
    ) {
        self.state.with_provider(&self.cap, |view| {
            load_reasoning_effort_picker_items(view);
        });
    }
}

impl Message<LoadEndpointPickerEntries> for ProviderActor {
    type Reply = ();

    async fn handle(
        &mut self,
        _msg: LoadEndpointPickerEntries,
        _ctx: &mut MsgContext<Self, Self::Reply>,
    ) {
        self.handle_load_endpoint_picker_entries(false).await;
    }
}

impl Message<RefreshEndpointPickerEntries> for ProviderActor {
    type Reply = ();

    async fn handle(
        &mut self,
        _msg: RefreshEndpointPickerEntries,
        _ctx: &mut MsgContext<Self, Self::Reply>,
    ) {
        self.handle_load_endpoint_picker_entries(true).await;
    }
}

impl Message<ModelsRefreshed> for ProviderActor {
    type Reply = ();

    async fn handle(&mut self, msg: ModelsRefreshed, _ctx: &mut MsgContext<Self, Self::Reply>) {
        self.handle_models_refreshed(&msg);
    }
}

impl Message<ModelCacheLoaded> for ProviderActor {
    type Reply = ();

    async fn handle(&mut self, msg: ModelCacheLoaded, _ctx: &mut MsgContext<Self, Self::Reply>) {
        self.handle_model_cache_loaded(&msg.cache);
    }
}

impl BusPublish for ProviderActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        &self.deps.services.bus
    }
}

impl ProviderActor {
    /// ProviderSwitch: update session profile and emit ProviderSwitched event.
    fn handle_provider_switch(&self, payload: &ProviderSwitch) {
        self.state.with_session(&self.session_cap, |view| {
            view.session
                .map()
                .get_or_create(&payload.session_id)
                .set_model(payload.provider_id.clone());
        });
    }

    /// ModelsRefreshed: update model cache and reload provider picker entries.
    fn handle_models_refreshed(&self, event: &ModelsRefreshed) {
        let now = jiff::Timestamp::now();
        let mut cache = crate::feat::provider_infra::ModelCache {
            entries: event.results.clone(),
            last_updated_at: Some(now),
        };
        {
            let registry = self.deps.services.provider_registry.read();
            merge_context_lengths_from_registry(&mut cache, &registry);
        }
        let models_dev = crate::feat::provider_infra::ModelsDevData::load(
            &self.deps.services.paths.models_dev_user_path(),
            &self.deps.services.paths.models_dev_system_path(),
        );
        merge_models_dev_data(&mut cache, &models_dev);
        // Merge remote models into the registry so create_factory() can find them.
        self.deps.services.provider_registry.merge_cache(&cache);
        self.state.with_provider(&self.cap, |view| {
            view.provider.set_model_cache(Some(cache));
            // Also reload provider picker entries from updated model cache.
            load_provider_picker_items(&self.deps.services, view);
        });
    }

    /// ModelCacheLoaded: restore model cache from disk and reload picker entries.
    fn handle_model_cache_loaded(&self, cache: &crate::feat::provider_infra::ModelCache) {
        let mut cache = cache.clone();
        {
            let registry = self.deps.services.provider_registry.read();
            merge_context_lengths_from_registry(&mut cache, &registry);
        }
        let models_dev = crate::feat::provider_infra::ModelsDevData::load(
            &self.deps.services.paths.models_dev_user_path(),
            &self.deps.services.paths.models_dev_system_path(),
        );
        merge_models_dev_data(&mut cache, &models_dev);
        // Merge remote models into the registry so create_factory() can find them.
        self.deps.services.provider_registry.merge_cache(&cache);
        self.state.with_provider(&self.cap, |view| {
            view.provider.set_model_cache(Some(cache));
            load_provider_picker_items(&self.deps.services, view);
        });
    }

    /// Resolve the active model's backend and either serve OpenRouter routing
    /// endpoints from the in-memory cache or fetch them, then write the picker
    /// entries, fetch timestamp, and clear the loading flag.
    ///
    /// `force` is true for `<c-r>` refresh (always re-fetch) and false for picker
    /// open (serve from cache when present). The backend gate lives here (the
    /// actor owns `Services`); the picker-open validator only checks `Single`.
    ///
    /// Every terminal branch writes back all three: items, fetched_at, and
    /// loading=false — so the spinner never sticks on success, error, or the
    /// non-OpenRouter placeholder path.
    async fn handle_load_endpoint_picker_entries(&mut self, force: bool) {
        // Snapshot what we need under the read lock, then release it before
        // the async network fetch (the view guard is `Send` but not held
        // across `.await` of a network call in practice; clone out instead).
        let (model, pinned, theme) = {
            let s = self.state.read();
            let session = s.session.active_session();
            let model = session.profile().model.clone();
            let pinned = session.profile().endpoint.clone();
            let theme = s.frontend.theme.clone();
            (model, pinned, theme)
        };

        let Some(target) = resolve_openrouter_target(&self.deps.services, &model) else {
            // Not served via OpenRouter (or an alloy): render the placeholder,
            // clear loading, and leave both the cache and fetched_at untouched
            // (this path never fetched anything).
            let entries = unavailable_endpoint_entries(theme, pinned.as_ref());
            self.state.with_provider(&self.cap, |view| {
                set_endpoint_picker_items(view, entries);
                view.provider_frontend.set_endpoint_loading(false);
            });
            return;
        };

        let key = target.model_id().to_owned();

        // Cache hit on a non-forced open: rebuild entries from the cached
        // upstream list (theme/pin re-derived), no network call.
        if !force && let Some((endpoints, ts)) = self.endpoints_cache.get(&key).cloned() {
            let entries = build_endpoint_entries(&endpoints, &theme, pinned.as_ref());
            self.state.with_provider(&self.cap, |view| {
                set_endpoint_picker_items(view, entries);
                view.provider_frontend.set_endpoint_fetched_at(Some(ts));
                view.provider_frontend.set_endpoint_loading(false);
            });
            return;
        }

        // Cache miss or forced refresh: fetch, store on success, build.
        let now = jiff::Timestamp::now();
        let (entries, fetched_at) = match fetch_endpoints(&target).await {
            Ok(endpoints) => {
                self.endpoints_cache.insert(key, (endpoints.clone(), now));
                (
                    build_endpoint_entries(&endpoints, &theme, pinned.as_ref()),
                    Some(now),
                )
            }
            // On error: sentinel only, cache untouched, keep prior fetched_at.
            Err(()) => (
                vec![EndpointEntry::auto_route(pinned.is_none(), theme)],
                None,
            ),
        };

        self.state.with_provider(&self.cap, |view| {
            set_endpoint_picker_items(view, entries);
            // Only stamp fetched_at on a successful fetch; on error leave it.
            if let Some(at) = fetched_at {
                view.provider_frontend.set_endpoint_fetched_at(Some(at));
            }
            view.provider_frontend.set_endpoint_loading(false);
        });
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

/// Merge `context_length` and input modalities from the models.dev reference
// data into the model cache, filling in `None` slots where neither the API nor
// `providers.toml` provided a value.
//
// This is the lowest-priority merge: only `None` entries are filled,
// and existing values from the API or `providers.toml` are never overwritten.
//
// Modality stamping is unconditional (idempotent `insert`): a stale on-disk
// cache that predates the modalities field gets re-enriched from models.dev
// on every load, so the Image bit is never permanently lost across upgrades.
fn merge_models_dev_data(
    cache: &mut crate::feat::provider_infra::ModelCache,
    models_dev: &crate::feat::provider_infra::ModelsDevData,
) {
    for models in cache.entries.values_mut() {
        for model in models.iter_mut() {
            models_dev.enrich(model);
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

    use crate::AppState;
    use crate::common::bus::test_harness::{TestHarness, await_recorded};

    use crate::common::state::State;
    use crate::feat::provider_infra::{
        InputModalities, ModelCache, ModelInfo, ProviderEntry, ProvidersConfig,
    };

    use super::{ModelCacheLoaded, ModelsRefreshed, ProviderActor, ProviderActorDeps};
    use crate::common::actor_deps::ActorDeps;
    use crate::feat::provider::protocol::command::LoadCompactionModelPickerEntries;
    use crate::feat::provider::protocol::command::LoadProviderPickerEntries;
    use crate::feat::provider::protocol::command::ProviderSwitch;
    use crate::feat::provider::protocol::event::ProviderSwitched;
    use crate::feat::session::model_selection::ModelSelection;
    use crate::feat::ui::picker_states::PickerExt;

    async fn create_harness() -> (TestHarness, State) {
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        (harness, state)
    }

    async fn spawn_actor(harness: &TestHarness, state: &State, deps: ActorDeps) {
        harness
            .spawn_actor::<ProviderActor>(ProviderActorDeps {
                deps,
                state: state.clone(),
                cap: crate::common::tcaps::mint::mint_provider_cap(),
                session_cap: crate::common::tcaps::mint::mint_session_cap(),
            })
            .await;
    }

    fn sample_config() -> ProvidersConfig {
        ProvidersConfig {
            providers: vec![ProviderEntry {
                model_info: Vec::new(),
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
        let (harness, state) = create_harness().await;
        let services = harness.actor_deps().await.services;
        let registry = crate::feat::provider_infra::ProviderRegistry::from_config(sample_config())
            .expect("registry");
        services.provider_registry.replace(registry);
        spawn_actor(&harness, &state, harness.actor_deps().await).await;

        let mut cache = ModelCache::new();
        cache.entries.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: Some(8192),
                input_modalities: InputModalities::text(),
            }],
        );
        cache.last_updated_at = Some(jiff::Timestamp::now());

        // When publishing ModelCacheLoaded via bus.
        harness
            .publish(ModelCacheLoaded {
                cache: cache.clone(),
            })
            .await;

        // Then the model cache is set in state.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            state.read().provider.model_cache.is_some(),
            "actor should have processed the event"
        );

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
        let (harness, state) = create_harness().await;
        let services = harness.actor_deps().await.services;
        let registry = crate::feat::provider_infra::ProviderRegistry::from_config(sample_config())
            .expect("registry");
        services.provider_registry.replace(registry);
        spawn_actor(&harness, &state, harness.actor_deps().await).await;

        let ts = jiff::Timestamp::now();
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );
        cache.last_updated_at = Some(ts);

        // When publishing ModelCacheLoaded via bus.
        harness
            .publish(ModelCacheLoaded {
                cache: cache.clone(),
            })
            .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Then the timestamp is preserved in state.
        let s = state.read();
        let loaded = s.provider.model_cache.as_ref().unwrap();
        assert!(loaded.last_updated_at.is_some());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_fills_context_length_from_registry_when_api_returns_none() {
        // Given a registry with zai provider that has context_length: Some(128_000).
        let config = ProvidersConfig {
            providers: vec![ProviderEntry {
                model_info: Vec::new(),
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
        let (harness, state) = create_harness().await;
        let deps = harness.actor_deps().await;
        let services = deps.services.clone();
        let registry =
            crate::feat::provider_infra::ProviderRegistry::from_config(config).expect("registry");
        services.provider_registry.replace(registry);
        spawn_actor(&harness, &state, deps).await;

        // When publishing ModelsRefreshed with zai model that has context_length: None.
        let mut results = std::collections::HashMap::new();
        results.insert(
            "zai".to_owned(),
            vec![ModelInfo {
                id: "zai-1.5".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );
        let event = ModelsRefreshed {
            session_id: state.read().session.active_session_id().clone(),
            results,
            errors: std::collections::HashMap::new(),
        };
        harness.publish(event).await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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
                model_info: Vec::new(),
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
        let (harness, state) = create_harness().await;
        let services = harness.actor_deps().await.services;
        let registry =
            crate::feat::provider_infra::ProviderRegistry::from_config(config).expect("registry");
        services.provider_registry.replace(registry);
        spawn_actor(&harness, &state, harness.actor_deps().await).await;

        // When publishing ModelsRefreshed where API returns context_length: Some(8192).
        let mut results = std::collections::HashMap::new();
        results.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: Some(8192),
                input_modalities: InputModalities::text(),
            }],
        );
        let event = ModelsRefreshed {
            session_id: state.read().session.active_session_id().clone(),
            results,
            errors: std::collections::HashMap::new(),
        };
        harness.publish(event).await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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
        // Given a provider actor.
        let (harness, state) = create_harness().await;
        spawn_actor(&harness, &state, harness.actor_deps().await).await;

        // When publishing ModelsRefreshed where API also returns context_length: None.
        let mut results = std::collections::HashMap::new();
        results.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );
        let event = ModelsRefreshed {
            session_id: state.read().session.active_session_id().clone(),
            results,
            errors: std::collections::HashMap::new(),
        };
        harness.publish(event).await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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
        // Given a provider actor.
        let (harness, state) = create_harness().await;
        spawn_actor(&harness, &state, harness.actor_deps().await).await;

        // When publishing ModelsRefreshed with results for groq (not in registry).
        let mut results = std::collections::HashMap::new();
        results.insert(
            "groq".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );
        let event = ModelsRefreshed {
            session_id: state.read().session.active_session_id().clone(),
            results,
            errors: std::collections::HashMap::new(),
        };
        harness.publish(event).await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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
                model_info: Vec::new(),
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
        let (harness, state) = create_harness().await;
        let deps = harness.actor_deps().await;
        let services = deps.services.clone();
        let registry =
            crate::feat::provider_infra::ProviderRegistry::from_config(config).expect("registry");
        services.provider_registry.replace(registry);
        spawn_actor(&harness, &state, deps).await;

        // When publishing ModelCacheLoaded with cache that has context_length: None.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "zai".to_owned(),
            vec![ModelInfo {
                id: "zai-1.5".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );
        cache.last_updated_at = Some(jiff::Timestamp::now());

        harness
            .publish(ModelCacheLoaded {
                cache: cache.clone(),
            })
            .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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
                model_info: Vec::new(),
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
        let (harness, state) = create_harness().await;
        let services = harness.actor_deps().await.services;
        let registry =
            crate::feat::provider_infra::ProviderRegistry::from_config(config).expect("registry");
        services.provider_registry.replace(registry);
        spawn_actor(&harness, &state, harness.actor_deps().await).await;

        // When publishing ModelCacheLoaded with cache that has context_length: Some(8192).
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "ollama".to_owned(),
            vec![ModelInfo {
                id: "llama3".to_owned(),
                context_length: Some(8192),
                input_modalities: InputModalities::text(),
            }],
        );
        cache.last_updated_at = Some(jiff::Timestamp::now());

        harness
            .publish(ModelCacheLoaded {
                cache: cache.clone(),
            })
            .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Then the cache value wins (8192), not the registry value (4096).
        let s = state.read();
        let loaded = s
            .provider
            .model_cache
            .as_ref()
            .expect("cache should be set");
        assert_eq!(loaded.entries["ollama"][0].context_length, Some(8192));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_from_models_dev_fills_none() {
        // Given a cache with context_length: None and models.dev data.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "zai".to_owned(),
            vec![ModelInfo {
                id: "glm-5.1".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );

        let mut models_dev = crate::feat::provider_infra::ModelsDevData::new();
        models_dev
            .context_lengths
            .insert("glm-5.1".to_owned(), 200_000);

        // When merging.
        super::merge_models_dev_data(&mut cache, &models_dev);

        // Then the model now has context_length from models.dev.
        assert_eq!(cache.entries["zai"][0].context_length, Some(200_000));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_from_models_dev_does_not_overwrite_existing() {
        // Given a cache with context_length: Some(100000) and models.dev has 200000.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "openai".to_owned(),
            vec![ModelInfo {
                id: "gpt-4o".to_owned(),
                context_length: Some(100_000),
                input_modalities: InputModalities::text(),
            }],
        );

        let mut models_dev = crate::feat::provider_infra::ModelsDevData::new();
        models_dev
            .context_lengths
            .insert("gpt-4o".to_owned(), 200_000);

        // When merging.
        super::merge_models_dev_data(&mut cache, &models_dev);

        // Then the existing value is preserved.
        assert_eq!(cache.entries["openai"][0].context_length, Some(100_000));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_from_models_dev_leaves_none_when_not_in_data() {
        // Given a cache with an unknown model.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "local".to_owned(),
            vec![ModelInfo {
                id: "my-custom-llama".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );

        let models_dev = crate::feat::provider_infra::ModelsDevData::new();

        // When merging with empty models.dev data.
        super::merge_models_dev_data(&mut cache, &models_dev);

        // Then it stays None.
        assert_eq!(cache.entries["local"][0].context_length, None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_priority_is_api_then_config_then_models_dev() {
        // Given three models with different source scenarios.
        let mut cache = ModelCache::new();
        // Model A: API returned a value.
        cache.entries.insert(
            "provider-a".to_owned(),
            vec![ModelInfo {
                id: "model-a".to_owned(),
                context_length: Some(100_000),
                input_modalities: InputModalities::text(),
            }],
        );
        // Model B: API returned None, config will fill it.
        cache.entries.insert(
            "provider-b".to_owned(),
            vec![ModelInfo {
                id: "model-b".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );
        // Model C: API returned None, no config, models.dev should fill it.
        cache.entries.insert(
            "provider-c".to_owned(),
            vec![ModelInfo {
                id: "model-c".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );
        // Model D: API returned None, no config, not in models.dev.
        cache.entries.insert(
            "provider-d".to_owned(),
            vec![ModelInfo {
                id: "model-d".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
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
        super::merge_models_dev_data(&mut cache, &models_dev);

        // Then: A keeps API value, B keeps config value, C gets models.dev, D stays None.
        assert_eq!(cache.entries["provider-a"][0].context_length, Some(100_000));
        assert_eq!(cache.entries["provider-b"][0].context_length, Some(64_000));
        assert_eq!(cache.entries["provider-c"][0].context_length, Some(300_000));
        assert_eq!(cache.entries["provider-d"][0].context_length, None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_from_models_dev_handles_multiple_providers() {
        // Given two providers with models that have None.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "zai".to_owned(),
            vec![ModelInfo {
                id: "glm-5.1".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );
        cache.entries.insert(
            "anthropic".to_owned(),
            vec![ModelInfo {
                id: "claude-sonnet-4-20250514".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
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
        super::merge_models_dev_data(&mut cache, &models_dev);

        // Then both providers get filled.
        assert_eq!(cache.entries["zai"][0].context_length, Some(200_000));
        assert_eq!(cache.entries["anthropic"][0].context_length, Some(200_000));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn merge_from_models_dev_stamps_image_bit_on_disk_loaded_cache() {
        // Given a stale cache (as loaded from disk) whose model is image-capable
        // in models.dev but carries only the text-only default.
        let mut cache = ModelCache::new();
        cache.entries.insert(
            "openrouter".to_owned(),
            vec![ModelInfo {
                id: "xiaomi/mimo-v2.5".to_owned(),
                context_length: None,
                input_modalities: InputModalities::text(),
            }],
        );
        let mut models_dev = crate::feat::provider_infra::ModelsDevData::new();
        models_dev
            .image_support
            .insert("xiaomi/mimo-v2.5".to_owned(), true);

        // When merging (the disk-load path re-enriches from models.dev).
        super::merge_models_dev_data(&mut cache, &models_dev);

        // Then the image bit is stamped despite the stale text-only cache.
        assert!(
            cache.entries["openrouter"][0]
                .input_modalities
                .contains(crate::feat::provider_infra::Modality::Image),
            "disk-loaded cache should gain the image bit via re-enrichment"
        );
    }
    #[rstest::rstest]
    #[tokio::test]
    async fn handle_dispatches_provider_switch_command() {
        // Given a provider actor.
        let (harness, state) = create_harness().await;
        let recorder = harness.spawn_recorder::<ProviderSwitched>().await;
        spawn_actor(&harness, &state, harness.actor_deps().await).await;
        let session_id = state.read().session.active_session_id().clone();

        // When publishing ProviderSwitch via bus.
        harness
            .publish(ProviderSwitch {
                session_id: session_id.clone(),
                provider_id: ModelSelection::Single("ollama/llama3".to_owned()),
            })
            .await;

        // Then the session model is updated.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_name, "ollama/llama3");

        let s = state.read();
        assert_eq!(
            s.session.active_session().profile().model,
            ModelSelection::Single("ollama/llama3".to_owned())
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_dispatches_load_provider_picker_entries_command() {
        // Given a provider actor with a registry.
        let (harness, state) = create_harness().await;
        let deps = harness.actor_deps().await;
        let registry = crate::feat::provider_infra::ProviderRegistry::from_config(sample_config())
            .expect("registry");
        deps.services.provider_registry.replace(registry);
        spawn_actor(&harness, &state, deps).await;

        // When publishing LoadProviderPickerEntries.
        harness.publish(LoadProviderPickerEntries).await;

        // Give the actor time to process.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

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
        // Given a provider actor with a registry.
        let (harness, state) = create_harness().await;
        let deps = harness.actor_deps().await;
        let registry = crate::feat::provider_infra::ProviderRegistry::from_config(sample_config())
            .expect("registry");
        deps.services.provider_registry.replace(registry);
        spawn_actor(&harness, &state, deps).await;

        // When publishing LoadCompactionModelPickerEntries.
        harness.publish(LoadCompactionModelPickerEntries).await;

        // Give the actor time to process.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Then the compaction model picker has entries (at least the sentinel).
        let s = state.read();
        let items = s.frontend.compaction_model_picker().items();
        assert!(
            !items.is_empty(),
            "compaction picker should have entries after loading"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn endpoint_load_for_non_openrouter_model_clears_loading_and_shows_placeholder() {
        // Given a provider actor whose registry has an ollama (non-OpenRouter) model,
        // and the loading flag pre-set as the open intent would.
        let (harness, state) = create_harness().await;
        let deps = harness.actor_deps().await;
        let registry = crate::feat::provider_infra::ProviderRegistry::from_config(sample_config())
            .expect("registry");
        deps.services.provider_registry.replace(registry);
        spawn_actor(&harness, &state, deps).await;

        state
            .write_test_no_cap()
            .active_session_mut()
            .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
        state.write_test_no_cap().frontend.pickers.endpoint_loading = true;

        // When publishing LoadEndpointPickerEntries.
        harness
            .publish(crate::feat::provider::protocol::command::LoadEndpointPickerEntries)
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Then loading is cleared (no stuck spinner) and a placeholder row shows.
        let s = state.read();
        assert!(
            !s.frontend.pickers.endpoint_loading,
            "non-OpenRouter load must clear the loading flag"
        );
        assert!(
            !s.frontend.endpoint_picker().items().is_empty(),
            "non-OpenRouter load must still show the placeholder row"
        );
    }
}
