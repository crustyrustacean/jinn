//! Provider slice — LLM provider selection, model discovery, and streaming UI.
//!
//! Provides [`ProviderState`] for tracking the active provider, model cache,
//! and provider picker state. Also contains the provider actor, discover actor,
//! and UI elements (streaming indicator, queue display).

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

use crate::actor::{Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink};
use crate::actor_host::{ActorSpawnResult, spawn_actor};
use nullslop_component::AppUiRegistry;
use nullslop_providers::NO_PROVIDER_ID;

use crate::PickerEntry;

/// Provider selection state — owned by the provider-actor.
///
/// Written to exclusively by `ProviderActor` and `IntentHandler`.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct ProviderState {
    /// The currently active provider. Always set — starts as `NO_PROVIDER_ID`.
    /// OWNER: provider-actor (sets on ProviderSwitch),
    ///        src/app.rs (sets initial value at startup).
    pub active_provider: String,

    /// Last known model cache from discovery.
    /// OWNER: provider-actor (updates from ModelsRefreshed event).
    pub model_cache: Option<nullslop_providers::ModelCache>,

    /// When the model list was last refreshed (UTC).
    /// OWNER: provider-actor (updates from ModelsRefreshed event).
    pub last_refreshed_at: Option<jiff::Timestamp>,

    /// Provider picker state (items, filter text, selection index).
    /// OWNER: provider-actor (loads entries via LoadPickerEntries),
    ///        IntentHandler (navigates picker, reads selected item).
    pub provider_picker: nullslop_selection_widget::SelectionState<PickerEntry>,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self {
            active_provider: NO_PROVIDER_ID.to_owned(),
            model_cache: None,
            last_refreshed_at: None,
            provider_picker: nullslop_selection_widget::SelectionState::new(),
        }
    }
}

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
