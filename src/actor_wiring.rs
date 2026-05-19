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
use nullslop_domain::common::actor::protocol::event::{
    ActorStarted, ActorStarting, AllActorsSpawned,
};
use nullslop_domain::feat::context::DefaultStrategyFactory;
use nullslop_domain::feat::context::strategy::token_estimator::TiktokenCounter;
use nullslop_domain::init::env_init_actor::{EnvInitActor, EnvInitActorDeps};
use nullslop_domain::init::provider_init_actor::{ProviderInitActor, ProviderInitActorDeps};
use nullslop_domain::init::system_ready_actor::{SystemReadyActor, SystemReadyActorDeps};
use nullslop_domain::strategy_registry::StrategyRegistryService;
use nullslop_domain::{
    ActorCounter, ActorHostService, ActorMessageSink, AppCore, AppMsg, InMemoryActorHost,
    MessageSink, ShutdownTracker, State, spawn, spawn_forwarding_task, system_spawn,
    wait_for_system_ready,
};

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
    session_store: SessionStoreService,
    user_preferences_storage: UserPreferencesStorageService,
) -> (AppCore, Services, ActorHostService) {
    // Create channel first — actors need the sender, but AppCore needs services
    // which needs the actor host which needs actors. Break the cycle by creating
    // the channel independently.
    let (sender, receiver) = kanal::unbounded::<AppMsg>();

    // Create the message sink that bridges actor output to AppCore's channel.
    let sink: Arc<dyn MessageSink> = Arc::new(ActorMessageSink::new(sender.clone()));

    // Create the actor counter — incremented by every spawn/system_spawn call.
    let counter = ActorCounter::new();

    // Create the shutdown tracker — shared across all actors for coordinated shutdown.
    let shutdown_tracker = ShutdownTracker::new();

    // Create shared State FIRST — injected into multiple actors.
    let state = State::new(AppState::default());

    // Set default CWD for sessions (inherited from shell).
    {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
        let mut guard = state.write();
        guard.session.default_cwd = cwd.clone();
        guard.active_session_mut().set_cwd(cwd);
        // Show welcome message on startup.
        guard
            .active_session_mut()
            .push_entry(nullslop_domain::welcome_msg());
    }

    // Build services (needed early for infrastructure actors).
    let strategy_registry = StrategyRegistryService::new(Arc::new(DefaultStrategyDiscovery));
    let paths = nullslop_domain::AppPaths::default();

    // Set themes directory from AppPaths (used by theme picker intent).
    {
        let mut guard = state.write();
        guard.frontend.themes_dir = paths.themes_dir();
        guard.frontend.system_themes_dir = paths.system_themes_dir();
    }
    let services = Services {
        paths: paths.clone(),
        handle: handle.clone(),
        actor_channel: ActorChannelService::new(sender.clone()),
        llm_service: llm_service.clone(),
        provider_registry: provider_registry.clone(),
        api_keys: api_keys.clone(),
        config_storage: config_storage.clone(),
        session_store: session_store.clone(),
        strategy_registry: strategy_registry.clone(),
        user_preferences_storage: user_preferences_storage.clone(),
    };

    // ── Infrastructure actors (no lifecycle events) ──────────────────────

    // System-ready actor: counts ActorStarted, signals main thread when done.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let system_ready_result = system_spawn::<SystemReadyActor>(
        "system-ready",
        sink.clone(),
        handle,
        &counter,
        &shutdown_tracker,
        SystemReadyActorDeps {
            ready_tx,
            counter: counter.clone(),
        },
    );

    // Emit lifecycle events for the infrastructure actor manually.
    let _ = sink.send_event(Event::ActorStarting(ActorStarting {
        name: "system-ready".to_owned(),
        description: Some("Counts ActorStarted events and signals system ready".to_owned()),
    }));
    let _ = sink.send_event(Event::ActorStarted(ActorStarted {
        name: "system-ready".to_owned(),
        description: Some("Counts ActorStarted events and signals system ready".to_owned()),
    }));

    // ── Init actors (self-schedule Initialize during activate) ────────────

    // Env init: loads providers.toml, resolves API keys, emits EnvironmentLoaded.
    let env_init_result = spawn::<EnvInitActor>(
        "env-init",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        EnvInitActorDeps {
            config_storage: config_storage.clone(),
            api_keys: api_keys.clone(),
        },
    );

    // Provider init: on EnvironmentLoaded, builds registry, merges cache, resolves last_model.
    let provider_init_result = spawn::<ProviderInitActor>(
        "provider-init",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        ProviderInitActorDeps {
            services: services.clone(),
            state: state.clone(),
        },
    );

    // Preferences: loads and persists user preferences.
    let prefs_result =
        spawn::<nullslop_domain::feat::preferences_actor::preferences_actor::PreferencesActor>(
            "preferences",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            nullslop_domain::feat::preferences_actor::preferences_actor::PreferencesActorDeps {
                storage: user_preferences_storage.clone(),
            },
        );

    // Preferences state sync: updates AppState from PreferencesUpdated events.
    let prefs_sync_result = spawn::<
        nullslop_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActor,
    >("preferences-sync", &sink, handle, &counter, &shutdown_tracker,
        nullslop_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActorDeps {
            state: state.clone(),
            paths: paths.clone(),
        },
    );

    // ── Domain actors ────────────────────────────────────────────────────

    // LLM streaming actor.
    let llm_result = spawn::<nullslop_domain::feat::llm_actor::LlmActor>(
        "llm-streaming",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::llm_actor::LlmActorDeps {
            factory: llm_service.clone(),
            services: Some(services.clone()),
            state: state.clone(),
        },
    );

    // Model discovery actor.
    let discover_result = spawn::<nullslop_domain::feat::provider::discover_actor::DiscoverActor>(
        "llm-provider-listing",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::provider::discover_actor::DiscoverActorDeps {
            registry: provider_registry.clone(),
            api_keys: api_keys.clone(),
            state: state.clone(),
            app_paths: paths.clone(),
        },
    );

    // Tool orchestrator actor.
    let orch_result = spawn::<nullslop_domain::feat::tools_actor::ToolOrchestratorActor>(
        "tool-orchestrator",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::tools_actor::ToolOrchestratorActorDeps {
            state: state.clone(),
            app_paths: paths.clone(),
        },
    );

    // Context / prompt assembly actor.
    let prompt_result = spawn::<nullslop_domain::feat::context::context_actor::PromptAssemblyActor>(
        "context",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::context::context_actor::PromptAssemblyActorDeps {
            state: state.clone(),
            factory: Some(Box::new(DefaultStrategyFactory)
                as Box<
                    dyn nullslop_domain::feat::context::strategy::types::StrategyFactory,
                >),
            services: services.clone(),
        },
    );

    // Session persistence actor.
    let token_counter = TiktokenCounter::o200k_base();
    let sp_result = spawn::<nullslop_domain::feat::session::session_actor::SessionPersistenceActor>(
        "session-persistence",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::session::session_actor::SessionPersistenceActorDeps {
            state: state.clone(),
            services: Some(services.clone()),
            store: Some(session_store.clone()),
            counter: token_counter,
        },
    );

    // Prompt scan actor.
    let scan_result = spawn::<nullslop_domain::feat::context::prompt_scan_actor::PromptScanActor>(
        "prompt-scan",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::ScanActorDeps {
            paths: services.paths.clone(),
            state: None,
        },
    );

    // Skills scan actor.
    let skills_result = spawn::<nullslop_domain::feat::skills::skills_scan_actor::SkillsScanActor>(
        "skills-scan",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::ScanActorDeps {
            paths: services.paths.clone(),
            state: Some(state.clone()),
        },
    );

    // Persona scan actor.
    let persona_scan_result = spawn::<
        nullslop_domain::feat::persona::persona_scan_actor::PersonaScanActor,
    >(
        "persona-scan",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::ScanActorDeps {
            paths: services.paths.clone(),
            state: None,
        },
    );

    // Provider actor.
    let prov_result = spawn::<nullslop_domain::feat::provider::provider_actor::ProviderActor>(
        "provider",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::provider::provider_actor::ProviderActorDeps {
            state: state.clone(),
            services: services.clone(),
        },
    );

    // Compaction actor.
    let compaction_result = spawn::<nullslop_domain::feat::compaction_actor::CompactionActor>(
        "compaction",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::compaction_actor::CompactionActorDeps {
            state: state.clone(),
            services: services.clone(),
            handle: handle.clone(),
        },
    );

    // Sidebar state actor — keeps sidebar cursor in sync after session removal.
    let sidebar_state_result =
        spawn::<nullslop_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActor>(
            "sidebar-state",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            nullslop_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActorDeps {
                state: state.clone(),
            },
        );

    // ── Build actor host ─────────────────────────────────────────────────

    let host = InMemoryActorHost::from_actors_with_handle(
        vec![
            system_ready_result,
            env_init_result,
            provider_init_result,
            prefs_result,
            prefs_sync_result,
            llm_result,
            discover_result,
            orch_result,
            prompt_result,
            sp_result,
            scan_result,
            skills_result,
            persona_scan_result,
            prov_result,
            compaction_result,
            sidebar_state_result,
        ],
        handle.clone(),
        shutdown_tracker,
    );
    let host_arc: Arc<dyn nullslop_domain::ActorHost> = Arc::new(host);

    // Spawn the async forwarding task — continuously drains AppMsg channel → actor host.
    let actor_host_service = ActorHostService::new(host_arc);
    spawn_forwarding_task(receiver, actor_host_service.clone(), handle);

    // Signal that all actors have been spawned.
    // SystemReadyActor waits for this before checking its count.
    let _ = sink.send_event(Event::AllActorsSpawned(AllActorsSpawned));

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
    let _ = sink.send_command(nullslop_domain::Command::RescanPersonas(
        nullslop_domain::feat::context::protocol::command::RescanPersonas,
    ));

    (core, services, actor_host_service)
}
