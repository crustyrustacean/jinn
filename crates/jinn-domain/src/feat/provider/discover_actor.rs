//! Model discovery actor - discovers available models from configured providers.
//!
//! Subscribes to `RefreshModels` commands and iterates over all configured
//! providers, calling each provider's `list_models()` endpoint for each.
//! Results are saved to disk as a [`ModelCache`] and emitted as a
//! `ModelsRefreshed` event.

use std::collections::HashMap;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::provider::protocol::command::RefreshModels;
use crate::feat::provider::protocol::event::ModelsRefreshed;
use crate::feat::provider_infra::ModelCache;
use crate::protocol::{Command, Event};
use error_stack::Report;
use jinn_provider::{
    Backend, LlmServiceError, ModelInfo, OpenAiCompatibleService, ProviderConfig,
    anthropic::AnthropicService, google::GoogleService,
};

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
    /// Runtime services.
    services: Services,
    /// Shared application state.
    state: State,
}

/// Dependencies for [`DiscoverActor`].
pub struct DiscoverActorDeps {
    /// Runtime services.
    pub services: Services,
    /// Shared application state.
    pub state: State,
}

impl Actor for DiscoverActor {
    type Message = NoDirectMsg;
    type Deps = DiscoverActorDeps;
    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Discovers available models");
        ctx.subscribe_command::<RefreshModels>();

        Self {
            services: deps.services,
            state: deps.state,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx).await,
            ActorEnvelope::Event(_) | ActorEnvelope::System(_) => {}
        }
    }
}

impl DiscoverActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        match command {
            Command::RefreshModels => {
                self.refresh_models(ctx).await;
            }
            _ => {}
        }
    }

    /// Discovers models for a single provider entry.
    ///
    /// Validates the entry (has models, valid backend, API key), constructs the
    /// appropriate service, calls `list_models`, and enriches with models.dev data.
    ///
    /// Returns `Ok(models)` on success, or `Err(error_message)` if the provider
    /// should be skipped.
    async fn discover_provider_models(
        entry: &crate::feat::provider_infra::ProviderEntry,
        api_keys: &crate::feat::provider_infra::ApiKeysService,
        models_dev: &crate::feat::provider_infra::ModelsDevData,
    ) -> Result<Vec<ModelInfo>, String> {
        let Some(placeholder_model) = entry.models.first() else {
            return Err("no models configured (skipping discovery)".to_owned());
        };

        let backend = entry
            .backend
            .parse::<Backend>()
            .map_err(|e| format!("invalid backend: {e}"))?;

        let api_key = if entry.requires_key {
            let Some(ref env_var) = entry.api_key_env else {
                return Err("requires_key but no api_key_env set".to_owned());
            };
            api_keys
                .get(env_var)
                .ok_or_else(|| "API key not resolved".to_owned())?
        } else {
            "dummy-key".to_owned()
        };

        let result: Result<Vec<ModelInfo>, Report<LlmServiceError>> = match backend {
            Backend::Anthropic => {
                let svc = AnthropicService::new(placeholder_model.clone(), api_key, None);
                svc.list_models().await
            }
            Backend::Google => {
                let svc = GoogleService::new(placeholder_model.clone(), api_key);
                svc.list_models().await
            }
            _ => {
                let config = ProviderConfig::from(&backend);
                let svc = OpenAiCompatibleService::new(
                    config,
                    placeholder_model.clone(),
                    entry.base_url.clone(),
                    api_key,
                    entry.extra_body.clone(),
                );
                svc.list_models().await
            }
        };

        let mut models = result.map_err(|e| {
            tracing::warn!(provider = %entry.name, err = %e, "list_models failed");
            format!("{e}")
        })?;

        // Apply models.dev context length fallback to models
        // that didn't get it from the provider API.
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
        Ok(models)
    }
    /// Iterates all providers, discovers models, saves cache, emits event.
    async fn refresh_models(&self, ctx: &ActorContext) {
        let entries = {
            let registry = self.services.provider_registry.read();
            registry.config().providers.clone()
        };

        let mut results: HashMap<String, Vec<ModelInfo>> = HashMap::new();
        let mut errors: HashMap<String, String> = HashMap::new();

        let models_dev = crate::feat::provider_infra::ModelsDevData::load(
            &self.services.paths.models_dev_user_path(),
            &self.services.paths.models_dev_system_path(),
        );

        for entry in &entries {
            match Self::discover_provider_models(entry, &self.services.api_keys, &models_dev).await
            {
                Ok(models) => {
                    results.insert(entry.name.clone(), models);
                }
                Err(e) => {
                    errors.insert(entry.name.clone(), e);
                }
            }
        }

        let cache = ModelCache {
            entries: results.clone(),
            last_updated_at: Some(jiff::Timestamp::now()),
        };
        let path = self.services.paths.cache_path();
        if let Err(e) = cache.save(&path) {
            tracing::warn!("failed to save model cache: {e:?}");
        }

        let session_id = self.state.read().session.active_session_id().clone();
        let _ = ctx.send_event(Event::ModelsRefreshed(ModelsRefreshed {
            session_id,
            results,
            errors,
        }));
    }
}
