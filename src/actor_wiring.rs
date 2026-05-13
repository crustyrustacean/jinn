//! Actor wiring — spawns all actors and assembles the actor host.
//!
//! This module encapsulates the one-time startup wiring: creating shared state,
//! spawning each actor via the unified [`spawn`]/[`system_spawn`] functions,
//! building the actor host, starting the forwarding task, and waiting for the
//! actor system to become ready. Called once from `App::dispatch`.
//!
//! # Spawn order
//!
//! 1. Infrastructure actors via [`system_spawn`] (no lifecycle events):
//!    - `system-ready` — counts `ActorStarted`, signals main thread
//!    - `shutdown-tracker` — tracks actors for shutdown coordination
//! 2. Lifecycle events emitted for both infrastructure actors
//! 3. Init actors via [`spawn`] (self-schedule on startup):
//!    - `env-init` — loads config, resolves API keys
//!    - `provider-init` — builds registry, merges cache, resolves `last_model`
//!    - `preferences` — loads user preferences
//! 4. Domain actors via [`spawn`]:
//!    - All remaining actors

use std::sync::Arc;

use nullslop_domain::ApiKeysService;
use nullslop_domain::AppState;
use nullslop_domain::ConfigStorageService;
use nullslop_domain::DefaultStrategyDiscovery;
use nullslop_domain::Event;
use nullslop_domain::LlmServiceFactoryService;
use nullslop_domain::ProviderRegistryService;
use nullslop_domain::Services;
use nullslop_domain::SessionStoreService;
use nullslop_domain::UserPreferencesStorageService;
use nullslop_domain::actor_channel::ActorChannelService;
use nullslop_domain::common::actor::protocol::event::{ActorStarted, ActorStarting};
use nullslop_domain::core_channel::CoreChannelService;
use nullslop_domain::feat::context::DefaultStrategyFactory;
use nullslop_domain::feat::context::strategy::token_estimator::TiktokenCounter;
use nullslop_domain::feat::session::JsonlSessionStore as DomainJsonlSessionStore;
use nullslop_domain::feat::session::SessionStoreService as DomainSessionStoreService;
use nullslop_domain::init::env_init_actor::EnvInitActor;
use nullslop_domain::init::provider_init_actor::ProviderInitActor;
use nullslop_domain::init::system_ready_actor::SystemReadyActor;
use nullslop_domain::strategy_registry::StrategyRegistryService;
use nullslop_domain::{
    ActorHostService, ActorMessageSink, AppCore, AppMsg, InMemoryActorHost, MessageSink, State,
    spawn, spawn_forwarding_task, system_spawn, wait_for_system_ready,
};

/// Total number of actors in the system.
/// Used by the system-ready actor to know when all actors have started.
const ACTOR_COUNT: usize = 15;

/// Creates an `AppCore` with all actors registered and the async forwarding task started.
///
/// After spawning all actors, blocks the calling thread until the actor system
/// signals readiness (or times out after 3 seconds).
pub fn create_core_with_actor_host(
    handle: &tokio::runtime::Handle,
    llm_service: LlmServiceFactoryService,
    provider_registry: ProviderRegistryService,
    api_keys: ApiKeysService,
    config_storage: ConfigStorageService,
    user_preferences_storage: UserPreferencesStorageService,
) -> (
    AppCore,
    Services,
    ActorHostService,
    kanal::Receiver<nullslop_domain::CoreNotification>,
) {
    // Create channel first — actors need the sender, but AppCore needs services
    // which needs the actor host which needs actors. Break the cycle by creating
    // the channel independently.
    let (sender, receiver) = kanal::unbounded::<AppMsg>();

    // Create the actor→core notification channel.
    let (core_notify_tx, core_notify_rx) = kanal::unbounded::<nullslop_domain::CoreNotification>();

    // Create the message sink that bridges actor output to AppCore's channel.
    let sink = Arc::new(ActorMessageSink::new(sender.clone()));

    // Create shared State FIRST — injected into multiple actors.
    let state = State::new(AppState::default());

    // Set default CWD on all sessions (inherited from shell).
    {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
        let mut guard = state.write();
        guard
            .session
            .sessions
            .values_mut()
            .for_each(|s| s.set_cwd(cwd.clone()));
    }

    // Build services (needed early for infrastructure actors).
    let strategy_registry = StrategyRegistryService::new(Arc::new(DefaultStrategyDiscovery));
    let services = Services {
        handle: handle.clone(),
        actor_channel: ActorChannelService::new(sender.clone()),
        core_channel: CoreChannelService::new(core_notify_tx),
        llm_service: llm_service.clone(),
        provider_registry: provider_registry.clone(),
        api_keys: api_keys.clone(),
        config_storage: config_storage.clone(),
        session_store: SessionStoreService::new(
            Arc::new(nullslop_domain::JsonlSessionStore::new()),
        ),
        strategy_registry: strategy_registry.clone(),
        user_preferences_storage: user_preferences_storage.clone(),
    };

    // ── Infrastructure actors (no lifecycle events) ──────────────────────

    // System-ready actor: counts ActorStarted, signals main thread when done.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let system_ready_result =
        system_spawn::<SystemReadyActor>("system-ready", sink.clone(), handle, |ctx| {
            ctx.set_data(ready_tx);
            ctx.set_data(ACTOR_COUNT);
        });

    // Shutdown tracker: tracks actor lifecycle for coordinated shutdown.
    let shutdown_result = system_spawn::<nullslop_domain::feat::shutdown_actor::ShutdownTrackerActor>(
        "shutdown-tracker",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_data(state.clone());
            ctx.set_data(services.clone());
        },
    );

    // Emit lifecycle events for both infrastructure actors manually.
    let _ = sink.send_event(Event::ActorStarting {
        payload: ActorStarting {
            name: "system-ready".to_owned(),
            description: Some("Counts ActorStarted events and signals system ready".to_owned()),
        },
    });
    let _ = sink.send_event(Event::ActorStarted {
        payload: ActorStarted {
            name: "system-ready".to_owned(),
            description: Some("Counts ActorStarted events and signals system ready".to_owned()),
        },
    });
    let _ = sink.send_event(Event::ActorStarting {
        payload: ActorStarting {
            name: "shutdown-tracker".to_owned(),
            description: Some("Tracks actor lifecycle for shutdown coordination".to_owned()),
        },
    });
    let _ = sink.send_event(Event::ActorStarted {
        payload: ActorStarted {
            name: "shutdown-tracker".to_owned(),
            description: Some("Tracks actor lifecycle for shutdown coordination".to_owned()),
        },
    });

    // ── Init actors (self-schedule Initialize during activate) ────────────

    // Env init: loads providers.toml, resolves API keys, emits EnvironmentLoaded.
    let env_init_result = spawn::<EnvInitActor>("env-init", sink.clone(), handle, |ctx| {
        ctx.set_description("Loads environment variables and API keys");
        ctx.set_data(config_storage.clone());
        ctx.set_data(api_keys.clone());
    });

    // Provider init: on EnvironmentLoaded, builds registry, merges cache, resolves last_model.
    let provider_init_result =
        spawn::<ProviderInitActor>("provider-init", sink.clone(), handle, |ctx| {
            ctx.set_description("Loads provider config, merges cache, resolves last_model");
            ctx.set_data(services.clone());
        });

    // Preferences: loads and persists user preferences.
    let prefs_result = spawn::<
        nullslop_domain::feat::preferences_actor::preferences_actor::PreferencesActor,
    >("preferences", sink.clone(), handle, |ctx| {
        ctx.set_description("Persists user preferences to nullslop.toml");
        ctx.set_data(user_preferences_storage.clone());
    });

    // ── Domain actors ────────────────────────────────────────────────────

    // Echo actor.
    let echo_result = spawn::<nullslop_domain::feat::echo_actor::EchoActor>(
        "echo",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_description("Echoes messages back");
        },
    );

    // LLM streaming actor.
    let llm_result = spawn::<nullslop_domain::feat::llm_actor::LlmActor>(
        "llm-streaming",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_description("LLM streaming with tool support");
            ctx.set_data(llm_service.clone());
        },
    );

    // Model discovery actor.
    let discover_result = spawn::<nullslop_domain::feat::provider::discover_actor::DiscoverActor>(
        "llm-provider-listing",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_description("Discovers available models");
            ctx.set_data(provider_registry.clone());
            ctx.set_data(api_keys.clone());
        },
    );

    // Tool orchestrator actor.
    let orch_result = spawn::<nullslop_domain::feat::tools_actor::ToolOrchestratorActor>(
        "tool-orchestrator",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_description("Dispatches and manages tool execution");
            ctx.set_data(state.clone());
        },
    );

    // Context / prompt assembly actor.
    let prompt_result = spawn::<nullslop_domain::feat::context::context_actor::PromptAssemblyActor>(
        "context",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_description("Context assembly, strategy management, pinning, and templates");
            ctx.set_data(state.clone());
            ctx.set_data(Box::new(DefaultStrategyFactory)
                as Box<
                    dyn nullslop_domain::feat::context::strategy::types::StrategyFactory,
                >);
            ctx.set_data(services.clone());
        },
    );

    // Session persistence actor.
    let domain_session_store = DomainJsonlSessionStore::new();
    let domain_session_store_service =
        DomainSessionStoreService::new(Arc::new(domain_session_store));
    let token_counter = TiktokenCounter::o200k_base();
    let sp_result = spawn::<nullslop_domain::feat::session::session_actor::SessionPersistenceActor>(
        "session-persistence",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_description("Persists session data to disk");
            ctx.set_data(state.clone());
            ctx.set_data(domain_session_store_service.clone());
            ctx.set_data(token_counter);
        },
    );

    // Prompt scan actor.
    let scan_result = spawn::<nullslop_domain::feat::context::prompt_scan_actor::PromptScanActor>(
        "prompt-scan",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_description("Scans and reloads prompt templates");
            ctx.set_data(nullslop_domain::prompts_dir());
        },
    );

    // Skills scan actor.
    let skills_result = spawn::<nullslop_domain::feat::skills::skills_scan_actor::SkillsScanActor>(
        "skills-scan",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_description("Scans and loads agent skills from ~/.agents/skills");
            ctx.set_data(nullslop_domain::feat::skills::skills_dir());
            ctx.set_data(state.clone());
        },
    );

    // Persona scan actor.
    let persona_scan_result = spawn::<
        nullslop_domain::feat::persona::persona_scan_actor::PersonaScanActor,
    >("persona-scan", sink.clone(), handle, |ctx| {
        ctx.set_description("Scans and loads persona files from ~/.config/nullslop/personas");
        ctx.set_data(nullslop_domain::personas_dir());
    });

    // Provider actor.
    let prov_result = spawn::<nullslop_domain::feat::provider::provider_actor::ProviderActor>(
        "provider",
        sink.clone(),
        handle,
        |ctx| {
            ctx.set_description("Manages provider selection, LLM factory, and model cache");
            ctx.set_data(state.clone());
            ctx.set_data(services.clone());
        },
    );

    // ── Build actor host ─────────────────────────────────────────────────

    let host = InMemoryActorHost::from_actors_with_handle(
        vec![
            system_ready_result,
            shutdown_result,
            env_init_result,
            provider_init_result,
            prefs_result,
            echo_result,
            llm_result,
            discover_result,
            orch_result,
            prompt_result,
            sp_result,
            scan_result,
            skills_result,
            persona_scan_result,
            prov_result,
        ],
        handle.clone(),
    );
    let host_arc: Arc<dyn nullslop_domain::ActorHost> = Arc::new(host);

    // Spawn the async forwarding task — continuously drains AppMsg channel → actor host.
    let actor_host_service = ActorHostService::new(host_arc);
    spawn_forwarding_task(receiver, actor_host_service.clone(), handle);

    // Wait for the actor system to become ready (3-second timeout).
    wait_for_system_ready(ready_rx, handle);

    // Build AppCore with shared state and sender only.
    let core = AppCore {
        state: state.clone(),
        sender: sender.clone(),
    };

    // Trigger initial skills scan.
    let _ = sink.send_command(nullslop_domain::Command::ScanSkills);

    // Trigger initial persona scan.
    let _ = sink.send_command(nullslop_domain::Command::RescanPersonas {
        payload: nullslop_domain::feat::context::protocol::command::RescanPersonas,
    });

    (core, services, actor_host_service, core_notify_rx)
}
