//! Top-level application state and dispatch.
//!
//! [`App`] is the root of the ownership hierarchy. It creates the tokio
//! runtime, builds shared [`Services`], and dispatches to the appropriate
//! [`Runner`] variant (TUI or headless).

use std::path::Path;
use std::sync::Arc;

use error_stack::{Report, ResultExt};
use nullslop_actor::{Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink};
use nullslop_actor_host::{ActorHostService, InMemoryActorHost, spawn_actor};
use nullslop_cli::Cli;
use nullslop_component::AppState;
use nullslop_component_core::Bus;
use nullslop_context::{DefaultStrategyFactory, StrategyFactory};
use nullslop_context_actor::PromptAssemblyActor;
use nullslop_core::{ActorMessageSink, AppCore, AppMsg, State};
use nullslop_echo::EchoActor;
use nullslop_llm::LlmActor;
use nullslop_llm_discover::DiscoverActor;
use nullslop_prompt_scan::PromptScanActor;
use nullslop_protocol::Event;
use nullslop_protocol::actor::{ActorStarted, ActorStarting};
use nullslop_providers::ApiKeys;
use nullslop_providers::ApiKeysService;
use nullslop_providers::ConfigStorageService;
use nullslop_providers::FilesystemConfigStorage;
use nullslop_providers::LlmServiceFactoryService;
use nullslop_providers::ModelCache;
use nullslop_providers::NoProvidersAvailableFactory;
use nullslop_providers::ProviderId;
use nullslop_providers::ProviderRegistry;
use nullslop_providers::ProviderRegistryService;
use nullslop_providers::cache_path;
use nullslop_services::Services;
use nullslop_services::strategy_registry::StrategyRegistryService;
use nullslop_session::{JsonlSessionStore, SessionStoreService};
use nullslop_session_actor::{SessionPersistenceActor, SessionPersistenceDirectMsg};
use nullslop_tool_orchestrator::ToolOrchestratorActor;
use tokio::runtime::Runtime;
use wherror::Error;

use crate::headless::HeadlessApp;
use crate::runner::Runner;

/// Error type for top-level application initialization.
#[derive(Debug, Error)]
#[error(debug)]
pub struct AppError;

/// Top-level application state.
///
/// Created once in `crate::main` and dispatched to whichever
/// runner handles the command. Owns the tokio runtime and delegates
/// to [`Runner`] variants.
pub struct App {
    /// The tokio runtime.
    runtime: Runtime,
}

impl App {
    /// Creates a new top-level app with a default multi-threaded runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the tokio runtime cannot be created.
    pub fn new() -> Result<Self, Report<AppError>> {
        let runtime = Runtime::new()
            .change_context(AppError)
            .attach("failed to create tokio runtime")?;
        Ok(Self { runtime })
    }

    /// Returns a handle to the tokio runtime for spawning tasks.
    #[must_use]
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    /// Dispatches the CLI command to the appropriate runner.
    ///
    /// # Errors
    ///
    /// Returns an error if the runner fails.
    pub fn dispatch(&mut self, cli: Cli) -> Result<(), Report<AppError>> {
        use nullslop_cli::cli::{Commands, HeadlessCommands};

        // Load config from providers.toml (auto-creates on first run).
        let config_storage =
            ConfigStorageService::new(Arc::new(FilesystemConfigStorage::default_path()));
        let provider_config = config_storage
            .load()
            .change_context(AppError)
            .attach("failed to load provider config")?;

        // Resolve API keys at startup from environment variables.
        let mut api_keys = ApiKeys::new();
        for provider in &provider_config.providers {
            if let Some(ref env_var) = provider.api_key_env
                && let Ok(value) = std::env::var(env_var)
                && !value.is_empty()
            {
                api_keys.insert(env_var.clone(), value);
            }
        }
        let resolved_api_keys = ApiKeysService::new(api_keys);

        // Build provider registry.
        let provider_registry = ProviderRegistryService::new(
            ProviderRegistry::from_config(provider_config).change_context(AppError)?,
        );

        // Determine initial provider and factory.
        let (llm_service, initial_provider) =
            resolve_initial_factory(&provider_registry, &resolved_api_keys);

        match cli.command.unwrap_or(Commands::Tui) {
            Commands::Tui => {
                let (core, services) = create_core_with_actor_host(
                    &self.handle(),
                    llm_service.clone(),
                    provider_registry.clone(),
                    resolved_api_keys.clone(),
                    config_storage.clone(),
                );
                core.state.write().active_provider = initial_provider;
                load_model_cache(&core, &cache_path());
                ensure_prompt_example();
                load_prompt_templates(&core, &nullslop_prompt_template::prompts_dir());

                // Resolve mouse selection config from environment.
                let mouse_selection = !matches!(std::env::var("NULLSLOP_MOUSE_SELECTION"), Ok(val) if val.eq_ignore_ascii_case("false") || val == "0");
                let tui_config = nullslop_tui::config::TuiConfig::new(mouse_selection);

                let runner = Runner::Tui(Box::new(nullslop_tui::TuiApp::new_with_core_and_config(
                    services, core, tui_config,
                )));
                runner.run().change_context(AppError)?;
            }
            Commands::Headless { command, .. } => {
                let (core, services) = create_core_with_actor_host(
                    &self.handle(),
                    llm_service.clone(),
                    provider_registry,
                    resolved_api_keys,
                    config_storage,
                );
                core.state.write().active_provider = initial_provider;
                load_model_cache(&core, &cache_path());
                ensure_prompt_example();
                load_prompt_templates(&core, &nullslop_prompt_template::prompts_dir());
                let mut headless = HeadlessApp::new(core, services);
                match command {
                    Some(HeadlessCommands::SendChat { message }) => {
                        headless.send_chat(&message).change_context(AppError)?;
                    }
                    Some(HeadlessCommands::Script { path }) => {
                        let file = std::fs::File::open(&path)
                            .change_context(AppError)
                            .attach("failed to open script file")?;
                        headless.run_script(file).change_context(AppError)?;
                    }
                    None => {}
                }
                let runner = Runner::Headless(Box::new(headless));
                runner.run().change_context(AppError)?;
            }
        }

        Ok(())
    }
}

/// Resolves the initial LLM factory and provider name at startup.
///
/// Tries the configured default provider first, then falls back to the
/// first available provider. If none are available, returns a
/// [`NoProvidersAvailableFactory`] that streams a helpful setup message.
fn resolve_initial_factory(
    registry: &ProviderRegistryService,
    api_keys: &ApiKeysService,
) -> (LlmServiceFactoryService, String) {
    let registry_guard = registry.read();
    let api_keys_guard = api_keys.read();

    // Try configured default.
    if let Some(id) = registry_guard.default_provider_id()
        && registry_guard.is_available(&id, &api_keys_guard)
        && let Ok(factory) = registry_guard.create_factory(&id, &api_keys_guard)
    {
        tracing::info!("using configured default provider: {}", id.as_str());
        return (
            LlmServiceFactoryService::new(Arc::from(factory)),
            id.to_string(),
        );
    }

    // Fallback: first available provider.
    for provider in registry_guard.providers() {
        let id = ProviderId::new(provider.name.clone());
        if registry_guard.is_available(&id, &api_keys_guard)
            && let Ok(factory) = registry_guard.create_factory(&id, &api_keys_guard)
        {
            tracing::info!("using first available provider: {}", provider.name);
            return (
                LlmServiceFactoryService::new(Arc::from(factory)),
                provider.name.clone(),
            );
        }
    }

    // No provider available — use the no-provider factory.
    tracing::warn!("no provider configured or available; use the picker to select one");
    (
        LlmServiceFactoryService::new(Arc::new(NoProvidersAvailableFactory)),
        nullslop_providers::NO_PROVIDER_ID.to_owned(),
    )
}

impl Default for App {
    fn default() -> Self {
        Self::new().expect("failed to create default App")
    }
}

/// Creates an `AppCore` with all components registered and the actor host started.
fn create_core_with_actor_host(
    handle: &tokio::runtime::Handle,
    llm_service: LlmServiceFactoryService,
    provider_registry: ProviderRegistryService,
    api_keys: ApiKeysService,
    config_storage: ConfigStorageService,
) -> (AppCore, Services) {
    // Create channel first — actors need the sender, but AppCore needs services
    // which needs the actor host which needs actors. Break the cycle by creating
    // the channel independently.
    let (sender, receiver) = kanal::unbounded::<AppMsg>();

    // Create the message sink that bridges actor output to AppCore's channel.
    let sink = Arc::new(ActorMessageSink::new(sender.clone()));

    // Create echo actor using two-phase startup.
    let (echo_tx, echo_rx) = kanal::unbounded::<ActorEnvelope<nullslop_echo::EchoDirectMsg>>();
    let echo_ref = ActorRef::new(echo_tx);
    let mut echo_ctx = ActorContext::new("echo", sink.clone());
    echo_ctx.set_description("Echoes messages back");
    let echo_actor = EchoActor::activate(&mut echo_ctx);
    let echo_result = spawn_actor("echo", echo_actor, &echo_ref, echo_rx, echo_ctx, handle);

    // Create LLM actor with data injection.
    let (llm_tx, llm_rx) = kanal::unbounded::<ActorEnvelope<nullslop_llm::LlmDirectMsg>>();
    let llm_ref = ActorRef::new(llm_tx);
    let mut llm_ctx = ActorContext::new("llm-streaming", sink.clone());
    llm_ctx.set_description("LLM streaming with tool support");
    llm_ctx.set_data(llm_service.clone());
    let llm_actor = LlmActor::activate(&mut llm_ctx);
    let llm_result = spawn_actor(
        "llm-streaming",
        llm_actor,
        &llm_ref,
        llm_rx,
        llm_ctx,
        handle,
    );

    // Create discover actor with data injection.
    let (discover_tx, discover_rx) =
        kanal::unbounded::<ActorEnvelope<nullslop_llm_discover::DiscoverDirectMsg>>();
    let discover_ref = ActorRef::new(discover_tx);
    let mut discover_ctx = ActorContext::new("llm-provider-listing", sink.clone());
    discover_ctx.set_description("Discovers available models");
    discover_ctx.set_data(provider_registry.clone());
    discover_ctx.set_data(api_keys.clone());
    let discover_actor = DiscoverActor::activate(&mut discover_ctx);
    let discover_result = spawn_actor(
        "llm-provider-listing",
        discover_actor,
        &discover_ref,
        discover_rx,
        discover_ctx,
        handle,
    );

    // Create tool orchestrator actor.
    let (orch_tx, orch_rx) =
        kanal::unbounded::<ActorEnvelope<nullslop_tool_orchestrator::ToolOrchestratorDirectMsg>>();
    let orch_ref = ActorRef::new(orch_tx);
    let mut orch_ctx = ActorContext::new("tool-orchestrator", sink.clone());
    orch_ctx.set_description("Dispatches and manages tool execution");
    let orch_actor = ToolOrchestratorActor::activate(&mut orch_ctx);
    let orch_result = spawn_actor(
        "tool-orchestrator",
        orch_actor,
        &orch_ref,
        orch_rx,
        orch_ctx,
        handle,
    );

    // Create prompt assembly actor.
    let (ctx_tx, ctx_rx) =
        kanal::unbounded::<ActorEnvelope<nullslop_context_actor::ContextDirectMsg>>();
    let ctx_ref = ActorRef::new(ctx_tx);
    let mut prompt_ctx = ActorContext::new("context", sink.clone());
    prompt_ctx.set_description("Assembles LLM prompts from chat history");
    prompt_ctx.set_data::<Box<dyn StrategyFactory>>(Box::new(DefaultStrategyFactory));
    let prompt_actor = PromptAssemblyActor::activate(&mut prompt_ctx);
    let prompt_result = spawn_actor(
        "context",
        prompt_actor,
        &ctx_ref,
        ctx_rx,
        prompt_ctx,
        handle,
    );

    // Create session store and persistence actor.
    let session_store = JsonlSessionStore::new();
    let session_store_service = SessionStoreService::new(Arc::new(session_store));

    let (sp_tx, sp_rx) = kanal::unbounded::<ActorEnvelope<SessionPersistenceDirectMsg>>();
    let sp_ref = ActorRef::new(sp_tx);
    let mut sp_ctx = ActorContext::new("session-persistence", sink.clone());
    sp_ctx.set_description("Persists session data to disk");
    sp_ctx.set_data(session_store_service.clone());
    let sp_actor = SessionPersistenceActor::activate(&mut sp_ctx);
    let sp_result = spawn_actor(
        "session-persistence",
        sp_actor,
        &sp_ref,
        sp_rx,
        sp_ctx,
        handle,
    );

    // Create prompt scan actor with injected path.
    let (scan_tx, scan_rx) =
        kanal::unbounded::<ActorEnvelope<nullslop_prompt_scan::PromptScanDirectMsg>>();
    let scan_ref = ActorRef::new(scan_tx);
    let mut scan_ctx = ActorContext::new("prompt-scan", sink.clone());
    scan_ctx.set_description("Scans and reloads prompt templates");
    scan_ctx.set_data(nullslop_prompt_template::prompts_dir());
    let scan_actor = PromptScanActor::activate(&mut scan_ctx);
    let scan_result = spawn_actor(
        "prompt-scan",
        scan_actor,
        &scan_ref,
        scan_rx,
        scan_ctx,
        handle,
    );

    // Emit lifecycle events.
    let _ = sink.send_event(Event::ActorStarting {
        payload: ActorStarting {
            name: "echo".to_string(),
            description: Some("Echoes messages back".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarted {
        payload: ActorStarted {
            name: "echo".to_string(),
            description: Some("Echoes messages back".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarting {
        payload: ActorStarting {
            name: "llm-streaming".to_string(),
            description: Some("LLM streaming with tool support".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarted {
        payload: ActorStarted {
            name: "llm-streaming".to_string(),
            description: Some("LLM streaming with tool support".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarting {
        payload: ActorStarting {
            name: "llm-provider-listing".to_string(),
            description: Some("Discovers available models".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarted {
        payload: ActorStarted {
            name: "llm-provider-listing".to_string(),
            description: Some("Discovers available models".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarting {
        payload: ActorStarting {
            name: "tool-orchestrator".to_string(),
            description: Some("Dispatches and manages tool execution".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarted {
        payload: ActorStarted {
            name: "tool-orchestrator".to_string(),
            description: Some("Dispatches and manages tool execution".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarting {
        payload: ActorStarting {
            name: "context".to_string(),
            description: Some("Assembles LLM prompts from chat history".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarted {
        payload: ActorStarted {
            name: "context".to_string(),
            description: Some("Assembles LLM prompts from chat history".to_string()),
        },
    });
    // Session persistence lifecycle events.
    let _ = sink.send_event(Event::ActorStarting {
        payload: ActorStarting {
            name: "session-persistence".to_string(),
            description: Some("Persists session data to disk".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarted {
        payload: ActorStarted {
            name: "session-persistence".to_string(),
            description: Some("Persists session data to disk".to_string()),
        },
    });

    let _ = sink.send_event(Event::ActorStarting {
        payload: ActorStarting {
            name: "prompt-scan".to_string(),
            description: Some("Scans and reloads prompt templates".to_string()),
        },
    });
    let _ = sink.send_event(Event::ActorStarted {
        payload: ActorStarted {
            name: "prompt-scan".to_string(),
            description: Some("Scans and reloads prompt templates".to_string()),
        },
    });

    let host = InMemoryActorHost::from_actors_with_handle(
        vec![
            echo_result,
            llm_result,
            discover_result,
            orch_result,
            prompt_result,
            sp_result,
            scan_result,
        ],
        handle.clone(),
    );
    let host_arc: Arc<dyn nullslop_actor_host::ActorHost> = Arc::new(host);

    // Build services with the actor host.
    let strategy_registry =
        StrategyRegistryService::new(Arc::new(nullslop_context::DefaultStrategyDiscovery));
    let services = Services {
        handle: handle.clone(),
        actor_host: ActorHostService::new(host_arc.clone()),
        llm_service,
        provider_registry,
        api_keys,
        config_storage,
        session_store: session_store_service.clone(),
        strategy_registry,
    };

    // Build AppCore with services stored separately from state.
    let mut core = AppCore {
        bus: Bus::new(),
        state: State::new(AppState::default()),
        services: services.clone(),
        sender,
        receiver,
        actor_host: Some(ActorHostService::new(host_arc)),
    };
    let mut registry = nullslop_component::AppUiRegistry::new();
    nullslop_component::register_all(&mut core.bus, &mut registry);

    (core, services)
}

/// Loads the model cache from disk into the application state.
///
/// Called once after core creation. Failures are logged but not fatal —
/// the cache is optional and will be populated on first refresh.
fn load_model_cache(core: &AppCore, path: &Path) {
    let cache = ModelCache::load(path).unwrap_or_else(|e| {
        tracing::warn!("failed to load model cache: {e:?}");
        None
    });
    if let Some(ref c) = cache {
        tracing::info!(providers = c.entries.len(), "loaded model cache");
    }
    let mut state = core.state.write();
    state.last_refreshed_at = cache.as_ref().and_then(|c| c.last_updated_at);
    state.model_cache = cache;
}

/// Ensures the prompts directory exists and contains an example template.
///
/// Called once after core creation. Failures are logged but not fatal —
/// the example is a convenience, not a requirement.
fn ensure_prompt_example() {
    if let Err(e) = nullslop_prompt_template::ensure_prompts_dir_with_example() {
        tracing::warn!("failed to ensure prompt example: {e:?}");
    }
}

/// Loads prompt templates from the given directory into the application state.
///
/// Called once after core creation. Failures are logged but not fatal —
/// an empty store is used when the directory is missing or unreadable.
fn load_prompt_templates(core: &AppCore, path: &Path) {
    let store =
        nullslop_prompt_template::PromptTemplateStore::load_from_dir(path).unwrap_or_else(|e| {
            tracing::warn!("failed to load prompt templates: {e:?}");
            nullslop_prompt_template::PromptTemplateStore::new()
        });
    tracing::info!(count = store.len(), "loaded prompt templates");
    core.state.write().prompt_templates = store;
}

#[cfg(test)]
mod tests {
    use clap_verbosity_flag::Verbosity;
    use nullslop_cli::cli::{Cli, Commands, HeadlessCommands};

    use super::*;

    fn test_cli(command: Option<Commands>) -> Cli {
        Cli {
            verbosity: Verbosity::new(0, 0),
            log_dir: None,
            command,
        }
    }

    #[rstest::rstest]
    fn dispatch_headless_script_completes_successfully() {
        // Given a script file containing "q".
        let dir = tempfile::tempdir().expect("temp dir");
        let script_path = dir.path().join("test.script");
        std::fs::write(&script_path, "q").expect("write script");

        let mut app = App::new().expect("create app");
        let cli = test_cli(Some(Commands::Headless {
            log_file: None,
            command: Some(HeadlessCommands::Script {
                path: script_path.to_str().expect("path to str").to_string(),
            }),
        }));

        // When dispatching the headless script command.
        let result = app.dispatch(cli);

        // Then it completes without error.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn dispatch_headless_script_returns_error_for_missing_file() {
        // Given a nonexistent script path.
        let mut app = App::new().expect("create app");
        let cli = test_cli(Some(Commands::Headless {
            log_file: None,
            command: Some(HeadlessCommands::Script {
                path: "/no/such/file.script".to_string(),
            }),
        }));

        // When dispatching the headless script command.
        let result = app.dispatch(cli);

        // Then an error is returned.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    fn load_prompt_templates_sets_count() {
        // Given a temp directory with a template file.
        let dir = tempfile::tempdir().expect("temp dir");
        let template_content =
            "+++\nname = \"test\"\ndescription = \"Test template\"\n+++\nTest body.";
        std::fs::write(dir.path().join("test.md"), template_content).expect("write template");

        let services = nullslop_services::Services::new();
        let core = AppCore::new(services);

        // When loading prompt templates from the temp directory.
        load_prompt_templates(&core, dir.path());

        // Then the template count is correct.
        let state = core.state.read();
        assert_eq!(state.prompt_templates.len(), 1);
    }

    #[rstest::rstest]
    fn load_prompt_templates_contains_template() {
        // Given a temp directory with a template file.
        let dir = tempfile::tempdir().expect("temp dir");
        let template_content =
            "+++\nname = \"test\"\ndescription = \"Test template\"\n+++\nTest body.";
        std::fs::write(dir.path().join("test.md"), template_content).expect("write template");

        let services = nullslop_services::Services::new();
        let core = AppCore::new(services);

        // When loading prompt templates from the temp directory.
        load_prompt_templates(&core, dir.path());

        // Then the template is findable by name.
        let state = core.state.read();
        assert!(state.prompt_templates.find_by_name("test").is_some());
    }

    #[rstest::rstest]
    fn load_model_cache_populates_state_from_file() {
        // Given a temp directory with a model cache file.
        let dir = tempfile::tempdir().expect("temp dir");
        let cache_path = dir.path().join("model_cache.json");
        let cache = nullslop_providers::ModelCache {
            entries: {
                let mut map = std::collections::HashMap::new();
                map.insert(
                    "ollama".to_owned(),
                    vec!["llama3".to_owned(), "mistral".to_owned()],
                );
                map
            },
            last_updated_at: None,
        };
        cache.save(&cache_path).expect("save cache");

        let services = nullslop_services::Services::new();
        let core = AppCore::new(services);

        // When loading the model cache from the temp file.
        load_model_cache(&core, &cache_path);

        // Then the cache is in state with the expected entries.
        let state = core.state.read();
        assert!(state.model_cache.is_some());
        let cached = state.model_cache.as_ref().expect("cache present");
        assert_eq!(cached.entries.len(), 1);
        assert_eq!(cached.entries["ollama"].len(), 2);
    }

    #[rstest::rstest]
    fn load_model_cache_uses_empty_when_file_missing() {
        // Given a path to a nonexistent file.
        let dir = tempfile::tempdir().expect("temp dir");
        let cache_path = dir.path().join("nonexistent.json");

        let services = nullslop_services::Services::new();
        let core = AppCore::new(services);

        // When loading the model cache from a missing file.
        load_model_cache(&core, &cache_path);

        // Then the cache is None in state.
        let state = core.state.read();
        assert!(state.model_cache.is_none());
    }
}
