//! Provider slice — provider selection, model discovery, and streaming indicator.
//!
//! Spawns two actors:
//!
//! - **Provider actor** — manages provider selection, LLM factory, and model cache.
//! - **Discover actor** — discovers available models from configured providers.
//!
//! Also registers two display-only UI elements:
//!
//! - **Streaming indicator** — animated throbber shown during sending/streaming.
//! - **Queue display** — dimmed "QUEUED:" entries for messages waiting in the queue.

pub mod actor;
pub mod discover;
pub mod entries;
pub mod indicator;
pub mod loader;
pub mod queue_element;
pub mod render;

pub use indicator::StreamingIndicatorElement;
pub use queue_element::QueueDisplayElement;

use std::sync::Arc;

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink};
use nullslop_actor_host::{spawn_actor, ActorSpawnResult};
use nullslop_component::AppUiRegistry;

/// Register provider UI elements.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StreamingIndicatorElement::new()));
    registry.register(Box::new(QueueDisplayElement));
}

/// Spawns the provider actor on the given tokio runtime.
///
/// Creates the actor's channel, context, and run loop. Injects shared
/// [`State`](nullslop_component::State) and [`Services`](nullslop_services::Services).
/// Returns the `ActorRef` for sending direct messages and the `ActorSpawnResult`
/// containing the routing entry and join handle.
pub fn spawn_provider_actor(
    state: nullslop_component::State,
    services: nullslop_services::Services,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<actor::ProviderDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<actor::ProviderDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("provider", sink);
    ctx.set_description("Manages provider selection, LLM factory, and model cache");
    ctx.set_data(state);
    ctx.set_data(services);
    let actor = actor::ProviderActor::activate(&mut ctx);
    let result = spawn_actor("provider", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}

/// Spawns the model discovery actor on the given tokio runtime.
///
/// Creates the actor's channel, context, and run loop. Injects the provider
/// registry and API keys service. Returns the `ActorRef` for sending direct
/// messages and the `ActorSpawnResult` containing the routing entry and join handle.
pub fn spawn_discover_actor(
    registry: nullslop_providers::ProviderRegistryService,
    api_keys: nullslop_providers::ApiKeysService,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<discover::DiscoverDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<discover::DiscoverDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("llm-provider-listing", sink);
    ctx.set_description("Discovers available models");
    ctx.set_data(registry);
    ctx.set_data(api_keys);
    let actor = discover::DiscoverActor::activate(&mut ctx);
    let result = spawn_actor("llm-provider-listing", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}
