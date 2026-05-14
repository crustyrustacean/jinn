//! Model discovery actor — discovers available models from configured providers.
//!
//! Subscribes to `RefreshModels` commands and iterates over all configured
//! providers, calling each provider's `list_models()` endpoint for each.
//! Results are saved to disk as a [`ModelCache`] and emitted as a
//! `ModelsRefreshed` event.

use std::collections::HashMap;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::provider::protocol::command::RefreshModels;
use crate::feat::provider::protocol::event::ModelsRefreshed;
use crate::feat::provider_infra::{
    ApiKeysService, ModelCache, ProviderRegistryService, cache_path,
};
use crate::protocol::{Command, Event};
use error_stack::Report;
use nullslop_provider::{
    Backend, LlmServiceError, OpenAiCompatibleService, ProviderConfig, anthropic::AnthropicService,
    google::GoogleService,
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
    /// Provider registry for looking up configured providers.
    registry: ProviderRegistryService,
    /// Resolved API keys for provider authentication.
    api_keys: ApiKeysService,
    /// Shared application state (to read active session ID).
    state: State,
}

impl Actor for DiscoverActor {
    type Message = NoDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "data is injected by the host before activate is called"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<RefreshModels>();

        let registry = ctx
            .take_data::<ProviderRegistryService>()
            .expect("ProviderRegistryService must be injected via ctx.set_data()");
        let api_keys = ctx
            .take_data::<ApiKeysService>()
            .expect("ApiKeysService must be injected via ctx.set_data()");
        let state = ctx
            .take_data::<State>()
            .expect("State must be injected via ctx.set_data()");

        Self {
            registry,
            api_keys,
            state,
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

    /// Iterates all providers, discovers models, saves cache, emits event.
    async fn refresh_models(&self, ctx: &ActorContext) {
        let entries = {
            let registry = self.registry.read();
            registry.config().providers.clone()
        };

        let mut results: HashMap<String, Vec<String>> = HashMap::new();
        let mut errors: HashMap<String, String> = HashMap::new();

        for entry in &entries {
            // Need a placeholder model for the builder — use the first static model.
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
                if let Some(key) = self.api_keys.get(env_var) {
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

            let result: Result<Vec<String>, Report<LlmServiceError>> = match backend {
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
        let path = cache_path();
        if let Err(e) = cache.save(&path) {
            tracing::warn!("failed to save model cache: {e:?}");
        }

        // Emit ModelsRefreshed event.
        let session_id = self.state.read().session.active_session.clone();
        let _ = ctx.send_event(Event::ModelsRefreshed(ModelsRefreshed {
            session_id,
            results,
            errors,
        }));
    }
}
