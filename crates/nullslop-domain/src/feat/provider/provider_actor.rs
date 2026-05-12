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
//! - `last_refreshed_at`
//! - `provider_picker` entries (via the loader)
//!
//! # Lock discipline
//!
//! All handlers follow the same pattern: acquire state lock → mutate → release →
//! then emit. Never hold the lock during emission.

use std::sync::Arc;

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::common::actor_host::{ActorSpawnResult, spawn_actor};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::provider::protocol::command::ProviderSwitch;
use crate::feat::provider::protocol::event::{ModelsRefreshed, ProviderSwitched};
use crate::protocol::system::LoadPickerEntries;
use crate::protocol::{Command, Event, PickerKind};

use super::loader::load_provider_picker_items;
use crate::feat::picker::strategy_entries::load_strategy_picker_items;

/// Direct message type (unused — the provider actor only responds to bus commands).
pub enum ProviderDirectMsg {}

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

impl Actor for ProviderActor {
    type Message = ProviderDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<ProviderSwitch>();
        ctx.subscribe_command::<LoadPickerEntries>();
        ctx.subscribe_event::<ModelsRefreshed>();

        ctx.set_description("Manages provider selection, LLM factory, and model cache");

        #[expect(
            clippy::expect_used,
            reason = "State injection is required at activation"
        )]
        let state = ctx
            .take_data::<State>()
            .expect("ProviderActor requires State injection");
        #[expect(
            clippy::expect_used,
            reason = "Services injection is required at activation"
        )]
        let services = ctx
            .take_data::<Services>()
            .expect("ProviderActor requires Services injection");

        Self { state, services }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx),
            ActorEnvelope::Event(event) => {
                if let Event::ModelsRefreshed { ref payload } = event {
                    self.handle_models_refreshed(payload);
                }
            }
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl ProviderActor {
    /// Dispatches a command to the appropriate handler.
    fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::ProviderSwitch { payload } => {
                self.handle_provider_switch(payload, ctx);
            }
            Command::LoadPickerEntries { payload } => {
                self.handle_load_picker_entries(payload);
            }
            // Commands NOT subscribed to — these should not arrive.
            Command::SendMessage { .. }
            | Command::SwitchPromptStrategy { .. }
            | Command::RestoreStrategyState { .. }
            | Command::PinChatEntry { .. }
            | Command::UnpinChatEntry { .. }
            | Command::EnqueueUserMessage { .. }
            | Command::SetChatInputText { .. }
            | Command::PushChatEntry { .. }
            | Command::CancelStream { .. }
            | Command::AssemblePrompt { .. }
            | Command::SendToLlmProvider { .. }
            | Command::RefreshModels
            | Command::RescanPromptTemplates
            | Command::RegisterTools { .. }
            | Command::ExecuteToolBatch { .. }
            | Command::ExecuteTool { .. }
            | Command::ProceedWithShutdown { .. }
            | Command::SessionLoadCompleted { .. }
            | Command::SessionLoadRequested { .. }
            | Command::ScanSkills => {}
        }
    }

    // --- Command handlers ---

    /// ProviderSwitch: update active provider, emit ProviderSwitched event,
    /// and swap the LLM factory so subsequent messages use the new provider.
    fn handle_provider_switch(&self, payload: &ProviderSwitch, ctx: &ActorContext) {
        {
            let mut state = self.state.write();
            state
                .provider
                .active_provider
                .clone_from(&payload.provider_id);
        }

        if let Err(e) = ctx.send_event(Event::ProviderSwitched {
            payload: ProviderSwitched {
                provider_name: payload.provider_id.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "provider-actor failed to emit ProviderSwitched");
        }

        // Swap the LLM factory to the newly selected provider.
        let provider_id = crate::feat::provider_infra::ProviderId::new(payload.provider_id.clone());
        let api_keys = self.services.api_keys.read();
        match self
            .services
            .provider_registry
            .create_factory(&provider_id, &api_keys)
        {
            Ok(factory) => {
                self.services
                    .llm_service
                    .swap(std::sync::Arc::from(factory));
                tracing::info!(
                    provider = %payload.provider_id,
                    "swapped LLM factory"
                );
            }
            Err(e) => {
                tracing::warn!(
                    err = ?e,
                    provider = %payload.provider_id,
                    "failed to create factory for provider; leaving existing factory in place"
                );
            }
        }
    }

    /// LoadPickerEntries: load entries based on picker kind.
    fn handle_load_picker_entries(&self, payload: &LoadPickerEntries) {
        match payload.kind {
            PickerKind::Provider => {
                let mut state = self.state.write();
                load_provider_picker_items(&self.services, &mut state);
            }
            PickerKind::ContextAssembly => {
                let mut state = self.state.write();
                load_strategy_picker_items(&self.services, &mut state);
            }
            PickerKind::Session | PickerKind::Keymap => {
                // Future: load from services or state as appropriate.
            }
        }
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
        state.provider.last_refreshed_at = Some(now);
        // Also reload provider picker entries from updated model cache.
        load_provider_picker_items(&self.services, &mut state);
    }
}

/// Spawns the provider actor on the given tokio runtime.
///
/// Creates the actor's channel, context, and run loop. Injects shared
/// [`State`](crate::common::state::State) and [`Services`](crate::common::services::Services).
/// Returns the `ActorRef` for sending direct messages and the `ActorSpawnResult`
/// containing the routing entry and join handle.
pub fn spawn_provider_actor(
    state: crate::common::state::State,
    services: crate::common::services::Services,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<ProviderDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<ProviderDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("provider", sink);
    ctx.set_description("Manages provider selection, LLM factory, and model cache");
    ctx.set_data(state);
    ctx.set_data(services);
    let actor = ProviderActor::activate(&mut ctx);
    let result = spawn_actor("provider", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::common::app_state::AppState;
    use crate::common::services::Services;
    use crate::common::state::State;
    use crate::feat::provider::protocol::command::ProviderSwitch;
    use crate::feat::provider::protocol::event::ModelsRefreshed;
    use crate::protocol::system::LoadPickerEntries;
    use crate::protocol::{Command, Event, PickerKind};

    use super::ProviderActor;

    /// Creates a test actor with a fresh AppState and fake services.
    fn create_actor() -> (ProviderActor, State, Arc<RecordingSink>, ActorContext) {
        create_actor_with_services(Services::new())
    }

    /// Creates a test actor with custom services.
    fn create_actor_with_services(
        services: Services,
    ) -> (ProviderActor, State, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("provider-actor", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(state.clone());
        ctx.set_data(services);
        let actor = ProviderActor::activate(&mut ctx);
        (actor, state, sink, ctx)
    }

    // --- ProviderSwitch ---

    #[rstest::rstest]
    #[tokio::test]
    async fn provider_switch_updates_active_provider() {
        // Given a provider actor.
        let (mut actor, state, sink, ctx) = create_actor();

        // When processing ProviderSwitch.
        actor
            .handle(
                ActorEnvelope::Command(Command::ProviderSwitch {
                    payload: ProviderSwitch {
                        provider_id: "ollama".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the active provider is updated.
        {
            let guard = state.read();
            assert_eq!(guard.provider.active_provider, "ollama");
        }

        // And a ProviderSwitched event was emitted.
        let events = sink.events();
        let found = events
            .iter()
            .any(|e| matches!(e, Event::ProviderSwitched { .. }));
        assert!(found, "expected ProviderSwitched event");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn provider_switch_leaves_factory_unchanged_for_unknown_provider() {
        // Given a provider actor with a fake factory.
        let (mut actor, _state, _sink, ctx) = create_actor();
        let name_before = actor.services.llm_service.name();

        // When switching to an unknown provider.
        actor
            .handle(
                ActorEnvelope::Command(Command::ProviderSwitch {
                    payload: ProviderSwitch {
                        provider_id: "nonexistent/unknown".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the factory name is unchanged.
        let name_after = actor.services.llm_service.name();
        assert_eq!(name_before, name_after);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn provider_switch_swaps_factory_for_valid_provider() {
        // Given a provider actor with a registry containing a sample provider.
        use crate::common::services::test_services::TestServices;
        use crate::feat::provider_infra::{ProviderEntry, ProvidersConfig};

        let config = ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "sample".to_owned(),
                backend: "sample".to_owned(),
                models: vec!["sample".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
            }],
            aliases: vec![],
            default_provider: None,
        };
        let services = TestServices::builder().with_providers(config).build();

        let (mut actor, _state, _sink, ctx) = create_actor_with_services(services);
        let name_before = actor.services.llm_service.name();

        // When switching to a known provider.
        actor
            .handle(
                ActorEnvelope::Command(Command::ProviderSwitch {
                    payload: ProviderSwitch {
                        provider_id: "sample/sample".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the factory name changes to the new provider.
        let name_after = actor.services.llm_service.name();
        assert_ne!(name_before, name_after);
        assert_eq!(name_after, "Sample");
    }

    // --- LoadPickerEntries ---

    #[rstest::rstest]
    #[tokio::test]
    async fn load_picker_entries_context_assembly_populates_strategy_picker() {
        // Given a provider actor.
        let (mut actor, state, _sink, ctx) = create_actor();

        // When processing LoadPickerEntries for ContextAssembly.
        actor
            .handle(
                ActorEnvelope::Command(Command::LoadPickerEntries {
                    payload: LoadPickerEntries {
                        kind: PickerKind::ContextAssembly,
                    },
                }),
                &ctx,
            )
            .await;

        // Then the context strategy picker has entries.
        let guard = state.read();
        let items = guard.frontend.context_strategy_picker.items();
        assert!(!items.is_empty(), "strategy picker should have entries");
    }

    // --- ModelsRefreshed (event) ---

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_updates_model_cache() {
        // Given a provider actor.
        let (mut actor, state, _sink, ctx) = create_actor();

        let mut results = std::collections::HashMap::new();
        results.insert(
            "Ollama".to_owned(),
            vec!["llama3".to_owned(), "mistral".to_owned()],
        );

        // When processing ModelsRefreshed event.
        actor
            .handle(
                ActorEnvelope::Event(Event::ModelsRefreshed {
                    payload: ModelsRefreshed {
                        results: results.clone(),
                        errors: std::collections::HashMap::new(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the model cache and last_refreshed_at are updated.
        let guard = state.read();
        let cache = guard
            .provider
            .model_cache
            .as_ref()
            .expect("model cache should be set");
        assert_eq!(cache.entries.get("Ollama").map(std::vec::Vec::len), Some(2));
        assert!(guard.provider.last_refreshed_at.is_some());
    }
}
