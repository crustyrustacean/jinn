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
use nullslop_domain::feat::context::strategy::token_estimator::TiktokenCounter;
use nullslop_domain::feat::workflow::workflow_actor::{WorkflowActor, WorkflowActorDeps};
use nullslop_domain::init::env_init_actor::{EnvInitActor, EnvInitActorDeps};
use nullslop_domain::init::provider_init_actor::{ProviderInitActor, ProviderInitActorDeps};
use nullslop_domain::init::system_ready_actor::{SystemReadyActor, SystemReadyActorDeps};

use nullslop_domain::{
    ActorCounter, ActorHostService, ActorMessageSink, AppCore, AppMsg, InMemoryActorHost,
    MessageSink, ShutdownTracker, State, spawn, spawn_forwarding_task, system_spawn,
    wait_for_system_ready,
};

/// Creates an `AppCore` with all actors registered and the async forwarding task started.
///
/// After spawning all actors, blocks the calling thread until the actor system
/// signals readiness (or times out after 3 seconds).
#[expect(
    clippy::too_many_arguments,
    reason = "TODO: refactor to options struct"
)]
pub fn create_core_with_actor_host(
    handle: &tokio::runtime::Handle,
    llm_service: LlmServiceFactoryService,
    provider_registry: ProviderRegistryService,
    api_keys: ApiKeysService,
    config_storage: ConfigStorageService,
    session_store: SessionStoreService,
    user_preferences_storage: UserPreferencesStorageService,
    bench_csv_path: Option<std::path::PathBuf>,
    bench_plan: Option<nullslop_bench::orchestrator::BenchPlan>,
    bench_artifact_dir: Option<std::path::PathBuf>,
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
        guard.session.set_default_cwd(cwd.clone());
        guard.active_session_mut().set_cwd(cwd);
    }

    // Build services (needed early for infrastructure actors).
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
        user_preferences_storage: user_preferences_storage.clone(),
    };

    // ── Infrastructure actors (no lifecycle events) ──────────────────────

    let mut actors = Vec::new();

    // System-ready actor: counts ActorStarted, signals main thread when done.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    actors.push(system_spawn::<SystemReadyActor>(
        "system-ready",
        sink.clone(),
        handle,
        &counter,
        &shutdown_tracker,
        SystemReadyActorDeps {
            ready_tx,
            counter: counter.clone(),
        },
    ));

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
    actors.push(spawn::<EnvInitActor>(
        "env-init",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        EnvInitActorDeps {
            config_storage: config_storage.clone(),
            api_keys: api_keys.clone(),
        },
    ));

    // Provider init: on EnvironmentLoaded, builds registry, merges cache, resolves last_model.
    actors.push(spawn::<ProviderInitActor>(
        "provider-init",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        ProviderInitActorDeps {
            services: services.clone(),
            state: state.clone(),
        },
    ));

    // Preferences: loads and persists user preferences.
    actors.push(spawn::<
        nullslop_domain::feat::preferences_actor::preferences_actor::PreferencesActor,
    >(
        "preferences",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::preferences_actor::preferences_actor::PreferencesActorDeps {
            storage: user_preferences_storage.clone(),
        },
    ));

    // Preferences state sync: updates AppState from PreferencesUpdated events.
    actors.push(spawn::<
        nullslop_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActor,
    >("preferences-sync", &sink, handle, &counter, &shutdown_tracker,
        nullslop_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActorDeps {
            state: state.clone(),
            paths: paths.clone(),
        },
    ));

    // ── Domain actors ────────────────────────────────────────────────────

    // LLM streaming actor.
    actors.push(spawn::<nullslop_domain::feat::llm_actor::LlmActor>(
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
    ));

    // Model discovery actor.
    actors.push(spawn::<
        nullslop_domain::feat::provider::discover_actor::DiscoverActor,
    >(
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
    ));

    // Tool orchestrator actor.
    actors.push(spawn::<
        nullslop_domain::feat::tools_actor::ToolOrchestratorActor,
    >(
        "tool-orchestrator",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::tools_actor::ToolOrchestratorActorDeps {
            state: state.clone(),
            app_paths: paths.clone(),
            builtin_filter: None,
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
        },
    ));

    // Session persistence actor.
    let token_counter = TiktokenCounter::o200k_base();
    actors.push(spawn::<
        nullslop_domain::feat::session::session_actor::SessionPersistenceActor,
    >(
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
            builtin_registry: {
                let mut registry =
                    nullslop_domain::feat::session_lifecycle::builtin::BuiltinRegistry::new();
                nullslop_bench::bench_tasks::register_bench_tasks(
                    &mut registry,
                    bench_artifact_dir.as_deref(),
                );
                registry
            },
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
        },
    ));

    // Prompt scan actor.
    actors.push(spawn::<
        nullslop_domain::feat::context::prompt_scan_actor::PromptScanActor,
    >(
        "prompt-scan",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::context::prompt_scan_actor::PromptScanActorDeps {
            paths: services.paths.clone(),
        },
    ));

    // Skills scan actor.
    actors.push(spawn::<
        nullslop_domain::feat::skills::skills_scan_actor::SkillsScanActor,
    >(
        "skills-scan",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::skills::skills_scan_actor::SkillsScanActorDeps {
            paths: services.paths.clone(),
            state: state.clone(),
        },
    ));

    // Persona scan actor.
    actors.push(spawn::<
        nullslop_domain::feat::persona::persona_scan_actor::PersonaScanActor,
    >(
        "persona-scan",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::persona::persona_scan_actor::PersonaScanActorDeps {
            paths: services.paths.clone(),
        },
    ));

    // Judge scan actor.
    actors.push(spawn::<
        nullslop_domain::feat::judge::judge_scan_actor::JudgeScanActor,
    >(
        "judge-scan",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::judge::judge_scan_actor::JudgeScanActorDeps {
            paths: services.paths.clone(),
        },
    ));

    // Judge coordinator actor.
    actors.push(spawn::<
        nullslop_domain::feat::judge::judge_coordinator_actor::JudgeCoordinatorActor,
    >(
        "judge-coordinator",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::judge::judge_coordinator_actor::JudgeCoordinatorActorDeps {
            state: state.clone(),
        },
    ));

    // Provider actor.
    actors.push(spawn::<
        nullslop_domain::feat::provider::provider_actor::ProviderActor,
    >(
        "provider",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::provider::provider_actor::ProviderActorDeps {
            state: state.clone(),
            services: services.clone(),
        },
    ));

    // Compaction actor.
    actors.push(spawn::<
        nullslop_domain::feat::compaction_actor::CompactionActor,
    >(
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
    ));

    // Queue actor — dispatches queued turns when sessions become idle.
    actors.push(spawn::<nullslop_domain::feat::queue_actor::QueueActor>(
        "queue",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::queue_actor::QueueActorDeps {
            state: state.clone(),
            counter: token_counter,
        },
    ));

    // Sidebar state actor — keeps sidebar cursor in sync after session removal.
    actors.push(spawn::<
        nullslop_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActor,
    >(
        "sidebar-state",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActorDeps {
            state: state.clone(),
        },
    ));

    // Build workflow registry and register built-in workflows.
    let workflow_registry = Arc::new({
        let mut registry = nullslop_domain::feat::workflow::WorkflowRegistry::new();
        nullslop_domain::feat::workflow::register_all_workflows(&mut registry);
        registry
    });

    // Workflow actor — bridges workflow engine to actor bus.
    actors.push(spawn::<WorkflowActor>(
        "workflow",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        WorkflowActorDeps {
            state: state.clone(),
            services: services.clone(),
            registry: workflow_registry,
        },
    ));

    // ── Bench actor (conditional) ─────────────────────────────────────────
    if bench_csv_path.is_some() {
        actors.push(spawn::<nullslop_bench::bench_actor::BenchActor>(
            "bench",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            nullslop_bench::bench_actor::BenchActorDeps {
                state: state.clone(),
                csv_path: bench_csv_path.clone(),
                plan: bench_plan,
            },
        ));
    }

    // Spawn the async forwarding task — continuously drains AppMsg channel → actor host.
    let actor_host_service = ActorHostService::new(Arc::new(
        InMemoryActorHost::from_actors_with_handle(actors, handle.clone(), shutdown_tracker),
    ));
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

    // Trigger initial judge scan.
    let _ = sink.send_command(nullslop_domain::Command::RescanJudges(
        nullslop_domain::feat::judge::RescanJudges,
    ));

    (core, services, actor_host_service)
}
