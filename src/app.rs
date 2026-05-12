//! Top-level application state and dispatch.
//!
//! [`App`] is the root of the ownership hierarchy. It creates the tokio
//! runtime, builds shared [`Services`], and dispatches to the appropriate
//! [`Runner`] variant (TUI or headless).

use std::path::Path;
use std::sync::Arc;

use error_stack::{Report, ResultExt};
use nullslop_cli::Cli;
use nullslop_domain::ApiKeys;
use nullslop_domain::ApiKeysService;
use nullslop_domain::AppState;
use nullslop_domain::ConfigStorageService;
use nullslop_domain::DefaultStrategyDiscovery;
use nullslop_domain::Event;
use nullslop_domain::FilesystemConfigStorage;
use nullslop_domain::LlmServiceFactoryService;
use nullslop_domain::MessageSink;
use nullslop_domain::ModelCache;
use nullslop_domain::NoProvidersAvailableFactory;
use nullslop_domain::ProviderRegistry;
use nullslop_domain::ProviderRegistryService;
use nullslop_domain::Services;
use nullslop_domain::SessionStoreService;
use nullslop_domain::actor_channel::ActorChannelService;
use nullslop_domain::cache_path;
use nullslop_domain::core_channel::CoreChannelService;
use nullslop_domain::feat::context::DefaultStrategyFactory;
use nullslop_domain::feat::session::JsonlSessionStore as DomainJsonlSessionStore;
use nullslop_domain::feat::session::SessionStoreService as DomainSessionStoreService;
use nullslop_domain::strategy_registry::StrategyRegistryService;
use nullslop_domain::{ActorHostService, InMemoryActorHost};
use nullslop_domain::{ActorMessageSink, AppCore, AppMsg, State, spawn_forwarding_task};
use nullslop_domain::{ActorStarted, ActorStarting};
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
                let (core, services, actor_host, core_receiver) = create_core_with_actor_host(
                    &self.handle(),
                    llm_service.clone(),
                    provider_registry.clone(),
                    resolved_api_keys.clone(),
                    config_storage.clone(),
                );
                core.state.write().provider.active_provider = initial_provider;
                load_model_cache(&core.state, &cache_path());
                ensure_prompt_example();
                load_prompt_templates(&core.state, &nullslop_domain::prompts_dir());

                // Resolve mouse selection config from environment.
                let mouse_selection = !matches!(std::env::var("NULLSLOP_MOUSE_SELECTION"), Ok(val) if val.eq_ignore_ascii_case("false") || val == "0");

                let tui_config = nullslop_tui::config::TuiConfig::new(mouse_selection);
                let mut ui_registry = nullslop_domain::AppUiRegistry::new();
                nullslop_domain::register_tui_elements(&mut ui_registry);
                nullslop_domain::feat::ui::status_bar::register(&mut ui_registry);
                nullslop_domain::feat::ui::char_counter::register(&mut ui_registry);
                nullslop_domain::feat::dashboard::register(&mut ui_registry);
                nullslop_domain::feat::ui::chat_log::register(&mut ui_registry);
                nullslop_domain::feat::provider::register(&mut ui_registry);
                nullslop_domain::feat::pinned_panel::register(&mut ui_registry);
                nullslop_domain::feat::chat_input::register(&mut ui_registry);
                let which_key = nullslop_tui::app::WhichKeyInstance::new(
                    nullslop_tui::keymap::init(),
                    nullslop_tui::Scope::Normal,
                );

                let runner = Runner::Tui(Box::new(nullslop_tui::TuiApp {
                    core,
                    services,
                    actor_host,
                    core_receiver,
                    ui_registry,
                    events: nullslop_tui::MsgHandler::new(),
                    which_key,
                    suspend: nullslop_tui::suspend::Suspend::new(),
                    event_task: None,
                    status: nullslop_tui::AppStatus::Starting,
                    tab_manager: nullslop_tui::render::init_tab_manager(),
                    selection: nullslop_tui::selection::SelectionState::Idle,
                    selectable_rects: Default::default(),
                    pending_clipboard: false,
                    config: tui_config,
                    split_manager: ratatui_spatial_splits::SplitManager::new(),
                    pane_focus: nullslop_tui::app::PaneFocus::Chat,
                    pinned_pane_visible: false,
                    pinned_pane_id: None,
                }));
                runner.run().change_context(AppError)?;
            }
            Commands::Headless { command, .. } => {
                let (core, _services, actor_host, core_receiver) = create_core_with_actor_host(
                    &self.handle(),
                    llm_service.clone(),
                    provider_registry,
                    resolved_api_keys,
                    config_storage,
                );
                core.state.write().provider.active_provider = initial_provider;
                load_model_cache(&core.state, &cache_path());
                ensure_prompt_example();
                load_prompt_templates(&core.state, &nullslop_domain::prompts_dir());
                let mut headless = HeadlessApp::new(core, actor_host, core_receiver, self.handle());
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
        if registry_guard.is_available(&provider.id, &api_keys_guard)
            && let Ok(factory) = registry_guard.create_factory(&provider.id, &api_keys_guard)
        {
            tracing::info!("using first available provider: {}", provider.id.as_str());
            return (
                LlmServiceFactoryService::new(Arc::from(factory)),
                provider.id.to_string(),
            );
        }
    }

    // No provider available — use the no-provider factory.
    tracing::warn!("no provider configured or available; use the picker to select one");
    (
        LlmServiceFactoryService::new(Arc::new(NoProvidersAvailableFactory)),
        nullslop_domain::NO_PROVIDER_ID.to_owned(),
    )
}

impl Default for App {
    fn default() -> Self {
        Self::new().expect("failed to create default App")
    }
}

/// Creates an `AppCore` with all actors registered and the async forwarding task started.
fn create_core_with_actor_host(
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
    let (_discover_ref, discover_result) = nullslop_domain::feat::provider::spawn_discover_actor(
        provider_registry.clone(),
        api_keys.clone(),
        sink.clone(),
        handle,
    );

    // --- Tool orchestrator actor ---
    let (_orch_ref, orch_result) =
        nullslop_domain::feat::tools_actor::spawn(state.clone(), sink.clone(), handle);

    // --- Prompt assembly actor ---
    let (_ctx_ref, prompt_result) = nullslop_domain::feat::context::spawn_context_actor(
        state.clone(),
        Box::new(DefaultStrategyFactory),
        sink.clone(),
        handle,
    );

    // --- Session persistence actor ---
    let domain_session_store = DomainJsonlSessionStore::new();
    let domain_session_store_service =
        DomainSessionStoreService::new(Arc::new(domain_session_store));
    let (_sp_ref, sp_result) = nullslop_domain::feat::session::spawn_session_actor(
        state.clone(),
        domain_session_store_service.clone(),
        sink.clone(),
        handle,
    );

    // --- Prompt scan actor ---
    let (_scan_ref, scan_result) = nullslop_domain::feat::context::spawn_prompt_scan_actor(
        nullslop_domain::prompts_dir(),
        sink.clone(),
        handle,
    );

    // Build services (needed by provider actor and shutdown tracker).
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

    // --- Provider actor ---
    let (_prov_ref, prov_result) = nullslop_domain::feat::provider::spawn_provider_actor(
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
    let core = AppCore { state, sender };
    let mut registry = nullslop_domain::AppUiRegistry::new();
    nullslop_domain::register_all(&mut registry);
    nullslop_domain::feat::ui::status_bar::register(&mut registry);
    nullslop_domain::feat::ui::char_counter::register(&mut registry);
    nullslop_domain::feat::dashboard::register(&mut registry);
    nullslop_domain::feat::ui::chat_log::register(&mut registry);
    nullslop_domain::feat::provider::register(&mut registry);
    nullslop_domain::feat::pinned_panel::register(&mut registry);
    nullslop_domain::feat::chat_input::register(&mut registry);

    (core, services, actor_host_service, core_notify_rx)
}

/// Loads the model cache from disk into the application state.
///
/// Called once after core creation. Failures are logged but not fatal —
/// the cache is optional and will be populated on first refresh.
fn load_model_cache(state: &State, path: &Path) {
    let cache = ModelCache::load(path).unwrap_or_else(|e| {
        tracing::warn!("failed to load model cache: {e:?}");
        None
    });
    if let Some(ref c) = cache {
        tracing::info!(providers = c.entries.len(), "loaded model cache");
    }
    let mut state = state.write();
    state.provider.last_refreshed_at = cache.as_ref().and_then(|c| c.last_updated_at);
    state.provider.model_cache = cache;
}

/// Ensures the prompts directory exists and contains an example template.
///
/// Called once after core creation. Failures are logged but not fatal —
/// the example is a convenience, not a requirement.
fn ensure_prompt_example() {
    if let Err(e) = nullslop_domain::ensure_prompts_dir_with_example() {
        tracing::warn!("failed to ensure prompt example: {e:?}");
    }
}

/// Loads prompt templates from the given directory into the application state.
///
/// Called once after core creation. Failures are logged but not fatal —
/// an empty store is used when the directory is missing or unreadable.
fn load_prompt_templates(state: &State, path: &Path) {
    let store = nullslop_domain::PromptTemplateStore::load_from_dir(path).unwrap_or_else(|e| {
        tracing::warn!("failed to load prompt templates: {e:?}");
        nullslop_domain::PromptTemplateStore::new()
    });
    tracing::info!(count = store.len(), "loaded prompt templates");
    state.write().context.prompt_templates = store;
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

        let state = State::new(AppState::default());

        // When loading prompt templates from the temp directory.
        load_prompt_templates(&state, dir.path());

        // Then the template count is correct.
        let state = state.read();
        assert_eq!(state.context.prompt_templates.len(), 1);
    }

    #[rstest::rstest]
    fn load_prompt_templates_contains_template() {
        // Given a temp directory with a template file.
        let dir = tempfile::tempdir().expect("temp dir");
        let template_content =
            "+++\nname = \"test\"\ndescription = \"Test template\"\n+++\nTest body.";
        std::fs::write(dir.path().join("test.md"), template_content).expect("write template");

        let state = State::new(AppState::default());

        // When loading prompt templates from the temp directory.
        load_prompt_templates(&state, dir.path());

        // Then the template is findable by name.
        let state = state.read();
        assert!(
            state
                .context
                .prompt_templates
                .find_by_name("test")
                .is_some()
        );
    }

    #[rstest::rstest]
    fn load_model_cache_populates_state_from_file() {
        // Given a temp directory with a model cache file.
        let dir = tempfile::tempdir().expect("temp dir");
        let cache_path = dir.path().join("model_cache.json");
        let cache = nullslop_domain::ModelCache {
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

        let state = State::new(AppState::default());

        // When loading the model cache from the temp file.
        load_model_cache(&state, &cache_path);

        // Then the cache is in state with the expected entries.
        let state = state.read();
        assert!(state.provider.model_cache.is_some());
        let cached = state.provider.model_cache.as_ref().expect("cache present");
        assert_eq!(cached.entries.len(), 1);
        assert_eq!(cached.entries["ollama"].len(), 2);
    }

    #[rstest::rstest]
    fn load_model_cache_uses_empty_when_file_missing() {
        // Given a path to a nonexistent file.
        let dir = tempfile::tempdir().expect("temp dir");
        let cache_path = dir.path().join("nonexistent.json");

        let state = State::new(AppState::default());

        // When loading the model cache from a missing file.
        load_model_cache(&state, &cache_path);

        // Then the cache is None in state.
        let state = state.read();
        assert!(state.provider.model_cache.is_none());
    }

    #[rstest::rstest]
    fn resolve_initial_factory_finds_keyless_provider_without_default() {
        // Given a registry with a keyless lmstudio provider (no default set).
        let config = nullslop_domain::ProvidersConfig {
            providers: vec![nullslop_domain::feat::provider_infra::ProviderEntry {
                name: "lmstudio".to_owned(),
                backend: "ollama".to_owned(),
                models: vec!["my-model".to_owned()],
                base_url: Some("http://localhost:1234/v1".to_owned()),
                api_key_env: None,
                requires_key: false,
            }],
            aliases: vec![],
            default_provider: None,
        };
        let registry = nullslop_domain::ProviderRegistry::from_config(config).expect("registry");
        let registry_service = nullslop_domain::ProviderRegistryService::new(registry);
        let api_keys_service =
            nullslop_domain::ApiKeysService::new(nullslop_domain::ApiKeys::new());

        // When resolving the initial factory.
        let (factory, name) = resolve_initial_factory(&registry_service, &api_keys_service);

        // Then a real factory is returned (not the no-provider sentinel).
        assert_ne!(name, nullslop_domain::NO_PROVIDER_ID);
        assert_eq!(name, "lmstudio/my-model");
        assert_ne!(factory.name(), "NoProvidersAvailable");
    }
}
