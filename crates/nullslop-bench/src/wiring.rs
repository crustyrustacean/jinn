//! Bench-specific actor wiring.
//!
//! A parameterized copy of the production [`create_core_with_actor_host`] that
//! accepts tool configuration and session CWD for bench isolation. Each
//! task/model pair gets a fresh actor system via [`create_bench_core`].

#![allow(dead_code, reason = "used by runner in phase 4")]
#![allow(clippy::too_many_lines, reason = "mirrors production wiring")]

use std::path::PathBuf;
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
use nullslop_domain::init::env_init_actor::{EnvInitActor, EnvInitActorDeps};
use nullslop_domain::init::provider_init_actor::{ProviderInitActor, ProviderInitActorDeps};
use nullslop_domain::init::system_ready_actor::{SystemReadyActor, SystemReadyActorDeps};
use nullslop_domain::feat::tools_actor::tool_types::ToolDefinition;
use nullslop_domain::{
    ActorCounter, ActorHostService, ActorMessageSink, AppCore, AppMsg,
    InMemoryActorHost, MessageSink, ShutdownTracker, State,
    coordinated_shutdown, spawn, spawn_forwarding_task, system_spawn,
    wait_for_system_ready,
};

use crate::task::CustomTool;

/// Configuration for creating a bench-specific actor system.
pub struct BenchWiringConfig {
    /// Root directory for `AppPaths::new_in()` — controls database location.
    pub bench_root: PathBuf,
    /// LLM service factory.
    pub llm_service: LlmServiceFactoryService,
    /// Provider registry for model validation and switching.
    pub provider_registry: ProviderRegistryService,
    /// Resolved API keys.
    pub api_keys: ApiKeysService,
    /// Config storage for provider configuration.
    pub config_storage: ConfigStorageService,
    /// Session store for persistence.
    pub session_store: SessionStoreService,
    /// User preferences storage.
    pub user_preferences_storage: UserPreferencesStorageService,
    /// Subset of built-in tool names to register. Empty = register all.
    pub builtin_tools: Vec<String>,
    /// Additional custom tools.
    pub custom_tools: Vec<CustomTool>,
    /// Working directory for the session (usually the fixture directory).
    pub session_cwd: PathBuf,
}

/// Shuts down a bench actor system gracefully.
pub fn shutdown_bench(
    actor_host: &ActorHostService,
    state: &State,
    handle: &tokio::runtime::Handle,
) {
    coordinated_shutdown(
        actor_host.backend(),
        state,
        handle,
        nullslop_domain::SHUTDOWN_TIMEOUT,
    );
}

/// Creates a fresh `AppCore` with all actors for a single bench task/model pair.
///
/// Differences from production wiring:
/// - Uses `AppPaths::new_in(bench_root)` for database isolation
/// - Sets session CWD to the provided fixture directory
/// - Omits the welcome message
/// - Registers only the specified subset of built-in tools + custom tools
pub fn create_bench_core(
    handle: &tokio::runtime::Handle,
    config: BenchWiringConfig,
) -> (AppCore, Services, ActorHostService) {
    let (sender, receiver) = kanal::unbounded::<AppMsg>();
    let sink: Arc<dyn MessageSink> = Arc::new(ActorMessageSink::new(sender.clone()));
    let counter = ActorCounter::new();
    let shutdown_tracker = ShutdownTracker::new();
    let state = State::new(AppState::default());

    // Set default CWD and active session CWD to the fixture directory.
    {
        let mut guard = state.write();
        guard.session.set_default_cwd(config.session_cwd.clone());
        guard.active_session_mut().set_cwd(config.session_cwd);
        // No welcome message in bench mode.
    }

    let paths = nullslop_domain::AppPaths::new_in(&config.bench_root);

    {
        let mut guard = state.write();
        guard.frontend.themes_dir = paths.themes_dir();
        guard.frontend.system_themes_dir = paths.system_themes_dir();
    }

    let services = Services {
        paths: paths.clone(),
        handle: handle.clone(),
        actor_channel: ActorChannelService::new(sender.clone()),
        llm_service: config.llm_service.clone(),
        provider_registry: config.provider_registry.clone(),
        api_keys: config.api_keys.clone(),
        config_storage: config.config_storage.clone(),
        session_store: config.session_store.clone(),
        user_preferences_storage: config.user_preferences_storage.clone(),
    };

    // ── Infrastructure actors ────────────────────────────────────────────

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

    let _ = sink.send_event(Event::ActorStarting(ActorStarting {
        name: "system-ready".to_owned(),
        description: Some("Counts ActorStarted events and signals system ready".to_owned()),
    }));
    let _ = sink.send_event(Event::ActorStarted(ActorStarted {
        name: "system-ready".to_owned(),
        description: Some("Counts ActorStarted events and signals system ready".to_owned()),
    }));

    // ── Init actors ──────────────────────────────────────────────────────

    let env_init_result = spawn::<EnvInitActor>(
        "env-init",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        EnvInitActorDeps {
            config_storage: config.config_storage.clone(),
            api_keys: config.api_keys.clone(),
        },
    );

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

    let prefs_result =
        spawn::<nullslop_domain::feat::preferences_actor::preferences_actor::PreferencesActor>(
            "preferences",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            nullslop_domain::feat::preferences_actor::preferences_actor::PreferencesActorDeps {
                storage: config.user_preferences_storage.clone(),
            },
        );

    let prefs_sync_result = spawn::<
        nullslop_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActor,
    >("preferences-sync", &sink, handle, &counter, &shutdown_tracker,
        nullslop_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActorDeps {
            state: state.clone(),
            paths: paths.clone(),
        },
    );

    // ── Domain actors ────────────────────────────────────────────────────

    let llm_result = spawn::<nullslop_domain::feat::llm_actor::LlmActor>(
        "llm-streaming",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::llm_actor::LlmActorDeps {
            factory: config.llm_service.clone(),
            services: Some(services.clone()),
            state: state.clone(),
        },
    );

    let discover_result = spawn::<nullslop_domain::feat::provider::discover_actor::DiscoverActor>(
        "llm-provider-listing",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::provider::discover_actor::DiscoverActorDeps {
            registry: config.provider_registry.clone(),
            api_keys: config.api_keys.clone(),
            state: state.clone(),
            app_paths: paths.clone(),
        },
    );

    // Tool orchestrator with filtered builtins + custom tools.
    let builtin_filter = if config.builtin_tools.is_empty() {
        None
    } else {
        Some(config.builtin_tools.clone())
    };

    let orch_result = spawn::<nullslop_domain::feat::tools_actor::ToolOrchestratorActor>(
        "tool-orchestrator",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::tools_actor::ToolOrchestratorActorDeps {
            state: state.clone(),
            app_paths: paths.clone(),
            builtin_filter,
        },
    );

    // Register custom tools after the orchestrator starts by sending
    // RegisterTools command. Custom tools use the Builtin registration path
    // (execute function) rather than the Actor path.
    // We'll handle this after the system is ready (below).

    let prompt_result = spawn::<nullslop_domain::feat::context::context_actor::PromptAssemblyActor>(
        "context",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::context::context_actor::PromptAssemblyActorDeps {
            state: state.clone(),
            services: services.clone(),
        },
    );

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
            store: Some(config.session_store.clone()),
            counter: token_counter,
        },
    );

    let scan_result = spawn::<nullslop_domain::feat::context::prompt_scan_actor::PromptScanActor>(
        "prompt-scan",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::context::prompt_scan_actor::PromptScanActorDeps {
            paths: services.paths.clone(),
        },
    );

    let skills_result = spawn::<nullslop_domain::feat::skills::skills_scan_actor::SkillsScanActor>(
        "skills-scan",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        nullslop_domain::feat::skills::skills_scan_actor::SkillsScanActorDeps {
            paths: services.paths.clone(),
            state: state.clone(),
        },
    );

    let persona_scan_result =
        spawn::<nullslop_domain::feat::persona::persona_scan_actor::PersonaScanActor>(
            "persona-scan",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            nullslop_domain::feat::persona::persona_scan_actor::PersonaScanActorDeps {
                paths: services.paths.clone(),
            },
        );

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

    let actor_host_service = ActorHostService::new(host_arc);
    spawn_forwarding_task(receiver, actor_host_service.clone(), handle);

    let _ = sink.send_event(Event::AllActorsSpawned(AllActorsSpawned));
    wait_for_system_ready(ready_rx, handle);

    // Register custom tools via command (after system is ready).
    if !config.custom_tools.is_empty() {
        let definitions: Vec<ToolDefinition> = config
            .custom_tools
            .iter()
            .map(|ct| ct.definition.clone())
            .collect();
        let _ = sink.send_command(nullslop_domain::Command::RegisterTools(
            nullslop_domain::RegisterTools {
                provider: "bench-custom".to_owned(),
                definitions,
            },
        ));
    }

    let core = AppCore {
        state: state.clone(),
        sender: sender.clone(),
    };

    // Trigger initial scans.
    let _ = sink.send_command(nullslop_domain::Command::ScanSkills);
    let _ = sink.send_command(nullslop_domain::Command::RescanPersonas(
        nullslop_domain::feat::context::protocol::command::RescanPersonas,
    ));

    (core, services, actor_host_service)
}
