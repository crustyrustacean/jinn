//! Model discovery actor — discovers available models from configured providers.
//!
//! Subscribes to `RefreshModels` commands and iterates over all configured
//! providers, calling the `llm` crate's `list_models()` endpoint for each.
//! Results are saved to disk as a [`ModelCache`] and emitted as a
//! `ModelsRefreshed` event.

use std::collections::HashMap;
use std::sync::Arc;

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::common::actor_host::{ActorSpawnResult, spawn_actor};
use crate::feat::provider::protocol::command::RefreshModels;
use crate::feat::provider::protocol::event::ModelsRefreshed;
use crate::feat::provider_infra::{
    ApiKeysService, ModelCache, ProviderRegistryService, cache_path,
};
use crate::protocol::{Command, Event};
use llm::builder::LLMBuilder;

/// Error type for model discovery failures.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct DiscoverError;

/// Direct message type for the discover actor.
///
/// Currently unused — the actor responds to bus commands only.
pub enum DiscoverDirectMsg {}

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
}

impl Actor for DiscoverActor {
    type Message = DiscoverDirectMsg;

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

        Self { registry, api_keys }
    }

    async fn handle(&mut self, msg: ActorEnvelope<DiscoverDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx).await,
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::Event(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
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

            let backend = match entry.backend.parse::<llm::builder::LLMBackend>() {
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

            // Build provider.
            let mut builder = LLMBuilder::new().backend(backend).model(placeholder_model);

            if let Some(ref url) = entry.base_url {
                builder = builder.base_url(url);
            }
            if let Some(key) = &api_key {
                builder = builder.api_key(key);
            }

            let provider = match builder.build() {
                Ok(p) => p,
                Err(e) => {
                    errors.insert(entry.name.clone(), format!("build failed: {e}"));
                    continue;
                }
            };

            // Call list_models.
            match provider.list_models(None).await {
                Ok(response) => {
                    let models = response.get_models();
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
        let _ = ctx.send_event(Event::ModelsRefreshed {
            payload: ModelsRefreshed { results, errors },
        });
    }
}

/// Spawns the model discovery actor on the given tokio runtime.
///
/// Creates the actor's channel, context, and run loop. Injects the provider
/// registry and API keys service. Returns the `ActorRef` for sending direct
/// messages and the `ActorSpawnResult` containing the routing entry and join handle.
pub fn spawn_discover_actor(
    registry: crate::feat::provider_infra::ProviderRegistryService,
    api_keys: crate::feat::provider_infra::ApiKeysService,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<DiscoverDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<DiscoverDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("llm-provider-listing", sink);
    ctx.set_description("Discovers available models");
    ctx.set_data(registry);
    ctx.set_data(api_keys);
    let actor = DiscoverActor::activate(&mut ctx);
    let result = spawn_actor("llm-provider-listing", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}
