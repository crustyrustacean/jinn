//! Actor wiring — spawns all actors and assembles the actor host.
//!
//! This module encapsulates the one-time startup wiring: creating shared state,
//! spawning each actor, emitting lifecycle events, building the actor host,
//! and starting the forwarding task. Called once from `App::dispatch`.

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
use nullslop_domain::actor_channel::ActorChannelService;
use nullslop_domain::core_channel::CoreChannelService;
use nullslop_domain::feat::context::DefaultStrategyFactory;
use nullslop_domain::feat::session::JsonlSessionStore as DomainJsonlSessionStore;
use nullslop_domain::feat::session::SessionStoreService as DomainSessionStoreService;
use nullslop_domain::strategy_registry::StrategyRegistryService;
use nullslop_domain::{ActorHostService, InMemoryActorHost};
use nullslop_domain::{
    ActorMessageSink, AppCore, AppMsg, MessageSink, State, spawn_forwarding_task,
};
use nullslop_domain::{ActorStarted, ActorStarting};

/// Creates an `AppCore` with all actors registered and the async forwarding task started.
pub fn create_core_with_actor_host(
    handle: &tokio::runtime::Handle,
    llm_service: LlmServiceFactoryService,
    provider_registry: ProviderRegistryService,
    api_keys: ApiKeysService,
    config_storage: ConfigStorageService,
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

    // --- Echo actor ---
    let (_echo_ref, echo_result) = nullslop_domain::feat::echo_actor::spawn(sink.clone(), handle);

    // --- LLM actor ---
    let (_llm_ref, llm_result) =
        nullslop_domain::feat::llm_actor::spawn(llm_service.clone(), sink.clone(), handle);

    // --- Discover actor ---
    let (_discover_ref, discover_result) =
        nullslop_domain::feat::provider::discover_actor::spawn_discover_actor(
            provider_registry.clone(),
            api_keys.clone(),
            sink.clone(),
            handle,
        );

    // --- Tool orchestrator actor ---
    let (_orch_ref, orch_result) =
        nullslop_domain::feat::tools_actor::spawn(state.clone(), sink.clone(), handle);

    // Build services (needed by provider actor, context actor, and shutdown tracker).
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
    };

    // --- Prompt assembly actor ---
    let (_ctx_ref, prompt_result) =
        nullslop_domain::feat::context::context_actor::spawn_context_actor(
            state.clone(),
            Box::new(DefaultStrategyFactory),
            services.clone(),
            sink.clone(),
            handle,
        );

    // --- Session persistence actor ---
    let domain_session_store = DomainJsonlSessionStore::new();
    let domain_session_store_service =
        DomainSessionStoreService::new(Arc::new(domain_session_store));
    let (_sp_ref, sp_result) = nullslop_domain::feat::session::session_actor::spawn_session_actor(
        state.clone(),
        domain_session_store_service.clone(),
        sink.clone(),
        handle,
    );

    // --- Prompt scan actor ---
    let (_scan_ref, scan_result) =
        nullslop_domain::feat::context::prompt_scan_actor::spawn_prompt_scan_actor(
            nullslop_domain::prompts_dir(),
            sink.clone(),
            handle,
        );

    // --- Skills scan actor ---
    let (_skills_ref, skills_result) = nullslop_domain::feat::skills::spawn_skills_scan_actor(
        nullslop_domain::feat::skills::skills_dir(),
        state.clone(),
        sink.clone(),
        handle,
    );

    // --- Persona scan actor ---
    let (_persona_scan_ref, persona_scan_result) =
        nullslop_domain::feat::persona::persona_scan_actor::spawn_persona_scan_actor(
            nullslop_domain::personas_dir(),
            sink.clone(),
            handle,
        );

    // --- Provider actor ---
    let (_prov_ref, prov_result) =
        nullslop_domain::feat::provider::provider_actor::spawn_provider_actor(
            state.clone(),
            services.clone(),
            sink.clone(),
            handle,
        );

    // --- Shutdown tracker actor ---
    let (_st_ref, st_result) = nullslop_domain::feat::shutdown_actor::spawn(
        state.clone(),
        services.clone(),
        sink.clone(),
        handle,
    );

    // Emit lifecycle events for all actors.
    let actor_names = [
        ("echo", "Echoes messages back"),
        ("llm-streaming", "LLM streaming with tool support"),
        ("llm-provider-listing", "Discovers available models"),
        ("tool-orchestrator", "Dispatches and manages tool execution"),
        (
            "context",
            "Context assembly, strategy management, pinning, and templates",
        ),
        ("session-persistence", "Persists session data to disk"),
        ("prompt-scan", "Scans and reloads prompt templates"),
        (
            "skills-scan",
            "Scans and loads agent skills from ~/.agents/skills",
        ),
        (
            "persona-scan",
            "Scans and loads persona files from ~/.config/nullslop/personas",
        ),
        (
            "provider",
            "Manages provider selection, LLM factory, and model cache",
        ),
        (
            "shutdown-tracker",
            "Tracks actor lifecycle for shutdown coordination",
        ),
    ];
    for (name, desc) in &actor_names {
        let _ = sink.send_event(Event::ActorStarting {
            payload: ActorStarting {
                name: name.to_string(),
                description: Some(desc.to_string()),
            },
        });
        let _ = sink.send_event(Event::ActorStarted {
            payload: ActorStarted {
                name: name.to_string(),
                description: Some(desc.to_string()),
            },
        });
    }

    let host = InMemoryActorHost::from_actors_with_handle(
        vec![
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
            st_result,
        ],
        handle.clone(),
    );
    let host_arc: Arc<dyn nullslop_domain::ActorHost> = Arc::new(host);

    // Spawn the async forwarding task — continuously drains AppMsg channel → actor host.
    let actor_host_service = ActorHostService::new(host_arc);
    spawn_forwarding_task(receiver, actor_host_service.clone(), handle);

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
