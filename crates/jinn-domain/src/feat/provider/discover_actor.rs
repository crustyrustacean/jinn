//! Model discovery actor - discovers available models from configured providers.
//!
//! Subscribes to `RefreshModels` commands and iterates over all configured
//! providers, calling each provider's `list_models()` endpoint for each.
//! Results are saved to disk as a [`ModelCache`] and emitted as a
//! `ModelsRefreshed` event.

use std::collections::HashMap;

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::state::State;
use crate::feat::provider::protocol::command::RefreshModels;
use crate::feat::provider::protocol::event::ModelsRefreshed;
use crate::feat::provider_infra::ModelCache;
use error_stack::Report;
use jinn_provider::{
    Backend, LlmServiceError, ModelInfo, OpenAiCompatibleService, ProviderConfig,
    anthropic::AnthropicService, google::GoogleService,
};
use kameo::prelude::{Actor, ActorRef, Context, Message};

/// Error type for model discovery failures.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct DiscoverError;

/// Model discovery actor.
///
/// On `RefreshModels`, iterates all provider entries from the registry,
/// builds an LLM provider for each, calls `list_models(None)`, and collects
/// results. Saves the cache to disk and emits `ModelsRefreshed`.
pub struct DiscoverActor {
    deps: ActorDeps,
    state: State,
}

/// Dependencies for spawning a [`DiscoverActor`].
pub struct DiscoverActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
    /// Shared application state.
    pub state: State,
}

impl Actor for DiscoverActor {
    type Args = DiscoverActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<RefreshModels>())
            .await;

        Ok(Self {
            deps: args.deps,
            state: args.state,
        })
    }
}

impl Message<RefreshModels> for DiscoverActor {
    type Reply = ();

    async fn handle(
        &mut self,
        _msg: RefreshModels,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.refresh_models().await;
    }
}

impl BusPublish for DiscoverActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        self.deps.bus()
    }
}

impl DiscoverActor {
    /// Iterates all providers, discovers models, saves cache, emits event.
    #[expect(clippy::too_many_lines, reason = "handler reads best as a single unit")]
    async fn refresh_models(&self) {
        let entries = {
            let registry = self.deps.services.provider_registry.read();
            registry.config().providers.clone()
        };

        let mut results: HashMap<String, Vec<ModelInfo>> = HashMap::new();
        let mut errors: HashMap<String, String> = HashMap::new();

        // Load models.dev reference data for context length fallback.
        let models_dev = crate::feat::provider_infra::ModelsDevData::load(
            &self.deps.services.paths.models_dev_user_path(),
            &self.deps.services.paths.models_dev_system_path(),
        );

        for entry in &entries {
            // Need a placeholder model for the builder - use the first static model.
            let Some(placeholder_model) = entry.models.first() else {
                errors.insert(
                    entry.name.clone(),
                    "no models configured (skipping discovery)".to_owned(),
                );
                continue;
            };

            let backend = match entry.backend.parse::<Backend>() {
                Ok(b) => b,
                Err(e) => {
                    errors.insert(entry.name.clone(), format!("invalid backend: {e}"));
                    continue;
                }
            };

            // Resolve API key.
            let api_key = if entry.requires_key {
                let Some(ref env_var) = entry.api_key_env else {
                    errors.insert(
                        entry.name.clone(),
                        "requires_key but no api_key_env set".to_owned(),
                    );
                    continue;
                };
                if let Some(key) = self.deps.services.api_keys.get(env_var) {
                    Some(key)
                } else {
                    errors.insert(entry.name.clone(), "API key not resolved".to_owned());
                    continue;
                }
            } else {
                Some("dummy-key".to_owned())
            };

            // Build provider and call list_models.
            let api_key_str = api_key.as_deref().unwrap_or("");

            let result: Result<Vec<ModelInfo>, Report<LlmServiceError>> = match backend {
                Backend::Anthropic => {
                    let svc = AnthropicService::new(
                        placeholder_model.clone(),
                        api_key_str.to_owned(),
                        None,
                    );
                    svc.list_models().await
                }
                Backend::Google => {
                    let svc = GoogleService::new(placeholder_model.clone(), api_key_str.to_owned());
                    svc.list_models().await
                }
                _ => {
                    let config = ProviderConfig::from(&backend);
                    let svc = OpenAiCompatibleService::new(
                        config,
                        placeholder_model.clone(),
                        entry.base_url.clone(),
                        api_key_str.to_owned(),
                        entry.extra_body.clone(),
                    );
                    svc.list_models().await
                }
            };

            match result {
                Ok(models) => {
                    // Apply models.dev context length fallback to models
                    // that didn't get it from the provider API.
                    let mut models = models;
                    for model in &mut models {
                        if model.context_length.is_none()
                            && let Some(ctx) = models_dev.get(&model.id)
                        {
                            model.context_length = Some(ctx);
                        }
                    }
                    tracing::info!(
                        provider = %entry.name,
                        count = models.len(),
                        "discovered models"
                    );
                    results.insert(entry.name.clone(), models);
                }
                Err(e) => {
                    tracing::warn!(provider = %entry.name, err = %e, "list_models failed");
                    errors.insert(entry.name.clone(), format!("{e}"));
                }
            }
        }

        // Save cache to disk.
        let cache = ModelCache {
            entries: results.clone(),
            last_updated_at: Some(jiff::Timestamp::now()),
        };
        let path = self.deps.services.paths.cache_path();
        if let Err(e) = cache.save(&path) {
            tracing::warn!("failed to save model cache: {e:?}");
        }

        // Emit ModelsRefreshed event.
        let session_id = self.state.read().session.active_session_id().clone();
        self.publish(ModelsRefreshed {
            session_id,
            results,
            errors,
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        clippy::string_slice,
        reason = "test code"
    )]

    use std::time::Duration;

    use crate::common::app_state::AppState;
    use crate::common::bus::test_harness::{TestHarness, await_recorded};
    use crate::common::state::State;
    use crate::feat::provider::protocol::command::RefreshModels;
    use crate::feat::provider::protocol::event::ModelsRefreshed;

    use super::{DiscoverActor, DiscoverActorDeps};

    #[tokio::test]
    async fn refresh_models_emits_models_refreshed_event() {
        // Given a discover actor with no configured providers.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        let _actor = harness
            .spawn_actor::<DiscoverActor>(DiscoverActorDeps {
                deps: harness.actor_deps().await,
                state: state.clone(),
            })
            .await;
        let recorder = harness.spawn_recorder::<ModelsRefreshed>().await;

        // When publishing RefreshModels.
        harness.publish(RefreshModels).await;

        // Then a ModelsRefreshed event is emitted (with empty results).
        let events = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
        assert_eq!(events.len(), 1, "should emit one ModelsRefreshed event");
        assert!(events[0].results.is_empty(), "no providers configured, so results are empty");
    }
}
