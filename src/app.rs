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
use nullslop_domain::ConfigStorageService;
use nullslop_domain::FilesystemConfigStorage;
use nullslop_domain::FilesystemUserPreferencesStorage;
use nullslop_domain::LlmServiceFactoryService;
use nullslop_domain::NoProvidersAvailableFactory;
use nullslop_domain::ProviderRegistry;
use nullslop_domain::ProviderRegistryService;
use nullslop_domain::SessionStoreService;
use nullslop_domain::SqliteSessionStore;
use nullslop_domain::State;
use nullslop_domain::UserPreferencesStorageService;
use tokio::runtime::Runtime;
use wherror::Error;

use crate::actor_wiring;
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
        use nullslop_cli::cli::{BenchCommands, Commands, HeadlessCommands};

        // Load config from providers.toml (auto-creates on first run).
        let config_storage =
            ConfigStorageService::new(Arc::new(FilesystemConfigStorage::default_path()));
        // API keys are resolved by the env-init actor.
        let resolved_api_keys = ApiKeysService::new(ApiKeys::new());

        // Provider registry is populated by the provider-init actor.
        // Start with an empty registry.
        let provider_registry = ProviderRegistryService::new(
            ProviderRegistry::from_config(nullslop_domain::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .change_context(AppError)?,
        );

        // Initial factory is the no-provider sentinel until actors resolve the real one.
        let llm_service = LlmServiceFactoryService::new(Arc::new(NoProvidersAvailableFactory));

        // Guard: --db-path cannot be used with bench subcommands.
        if matches!(&cli.command, Some(Commands::Bench { .. })) && cli.db_path.is_some() {
            return Err(Report::new(AppError)
                .attach("--db-path cannot be used with bench subcommands. Use 'bench tui <db_path>' instead"));
        }

        // Create the session store — uses --db-path if provided, otherwise
        // the platform default. The --db-path flag lets users point the TUI
        // at a bench database to inspect results after a bench run.
        let session_store = {
            let store = match &cli.db_path {
                Some(path) => SqliteSessionStore::open_or_create(path),
                None => SqliteSessionStore::new(),
            };
            SessionStoreService::new(Arc::new(store.expect("failed to create session store")))
        };

        match cli.command.unwrap_or(Commands::Tui) {
            Commands::Completions { shell } => {
                use clap::CommandFactory;
                let mut cmd = Cli::command();
                let name = cmd.get_name().to_string();
                clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
                return Ok(());
            }
            Commands::Tui => {
                let (core, services, actor_host) = actor_wiring::create_core_with_actor_host(
                    &self.handle(),
                    llm_service.clone(),
                    provider_registry.clone(),
                    resolved_api_keys.clone(),
                    config_storage.clone(),
                    session_store.clone(),
                    UserPreferencesStorageService::new(Arc::new(
                        FilesystemUserPreferencesStorage::default_path(),
                    )),
                    None,
                    None,
                    None,
                );
                let paths = &services.paths;
                load_prompt_templates(
                    &core.state,
                    &paths.prompts_dir(),
                    &paths.system_prompts_dir(),
                );
                load_compaction_prompt(
                    &core.state,
                    &paths.prompts_dir(),
                    &paths.system_prompts_dir(),
                )?;
                load_theme(&core.state, &paths.themes_dir(), &paths.system_themes_dir());

                // Resolve mouse selection config from environment.
                let mouse_selection = !matches!(std::env::var("NULLSLOP_MOUSE_SELECTION"), Ok(val) if val.eq_ignore_ascii_case("false") || val == "0");

                // Initialize plugin system.
                // Create the MsgHandler early so the plugin CommandSender routes
                // through the TUI message handler (where the welcome subscriber
                // intercept lives) instead of directly to the actor host.
                let tui_events = nullslop_tui::MsgHandler::new();
                let tui_event_sender = tui_events.sender();
                let (plugin_host, welcome_subscriber) = {
                    let cmd_sender = nullslop_plugin::CommandSender::new(
                        move |cmd: nullslop_domain::Command| {
                            tui_event_sender.send(nullslop_tui::msg::Msg::Command(cmd));
                        },
                    );
                    let welcome_sub = nullslop_plugin::WelcomeSubscriber::new(cmd_sender.clone());
                    match nullslop_plugin::PluginHost::new(cmd_sender) {
                        Ok(host) => (Some(host), Some(welcome_sub)),
                        Err(e) => {
                            tracing::error!(err = ?e, "failed to create plugin host");
                            (None, None)
                        }
                    }
                };

                // Load system plugins (/usr/share/nullslop/plugins).
                if let Some(ref host) = plugin_host {
                    let mut plugin_count = 0usize;

                    // System plugins (installed by package manager).
                    let system_dir = paths.system_plugins_dir();
                    if system_dir.is_dir() {
                        let infos = host.load_all(&system_dir);
                        plugin_count += infos.len();
                    }

                    // User plugins (~/.config/nullslop/plugins).
                    let user_dir = paths.plugins_dir();
                    if user_dir.is_dir() {
                        let infos = host.load_all(&user_dir);
                        plugin_count += infos.len();
                    }

                    if plugin_count > 0 {
                        tracing::info!(count = plugin_count, "loaded plugins");
                    }

                    // Fire app::started event.
                    tracing::info!("dispatching app::started event");
                    host.dispatch_event("app::started", &serde_json::Value::Null);
                }

                let tui_config = nullslop_tui::config::TuiConfig::new(mouse_selection);
                let mut ui_registry = nullslop_domain::AppUiRegistry::new();
                nullslop_domain::register_all_ui_elements(&mut ui_registry);
                let which_key = nullslop_tui::app::WhichKeyInstance::new(
                    nullslop_tui::keymap::init(),
                    nullslop_tui::Scope::Normal,
                );

                let runner = Runner::Tui(Box::new(nullslop_tui::TuiApp {
                    core,
                    services,
                    actor_host,
                    ui_registry,
                    events: tui_events,
                    which_key,
                    suspend: nullslop_tui::suspend::Suspend::new(),
                    event_thread: None,
                    status: nullslop_tui::AppStatus::Starting,
                    selection: nullslop_tui::selection::SelectionState::Idle,
                    selectable_rects: Default::default(),
                    pending_clipboard: false,
                    config: tui_config,
                    sidebar: {
                        let mut s = nullslop_domain::feat::ui::sidebar::Sidebar::new();
                        nullslop_domain::feat::ui::sidebar::register_sections(&mut s);
                        s
                    },
                    preview_cache: nullslop_tui::app::PreviewCache::new(),
                    plugin_host,
                    welcome_subscriber,
                }));
                runner.run().change_context(AppError)?;
            }
            Commands::Headless { command, .. } => {
                let (core, _services, actor_host) = actor_wiring::create_core_with_actor_host(
                    &self.handle(),
                    llm_service.clone(),
                    provider_registry,
                    resolved_api_keys,
                    config_storage,
                    session_store,
                    UserPreferencesStorageService::new(Arc::new(
                        FilesystemUserPreferencesStorage::default_path(),
                    )),
                    None,
                    None,
                    None,
                );
                load_prompt_templates(
                    &core.state,
                    &_services.paths.prompts_dir(),
                    &_services.paths.system_prompts_dir(),
                );
                load_compaction_prompt(
                    &core.state,
                    &_services.paths.prompts_dir(),
                    &_services.paths.system_prompts_dir(),
                )?;
                load_theme(
                    &core.state,
                    &_services.paths.themes_dir(),
                    &_services.paths.system_themes_dir(),
                );
                let mut headless = HeadlessApp::new(core, actor_host, self.handle());
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
            Commands::Fetch { subcommand } => {
                use nullslop_cli::cli::FetchCommands;

                match subcommand {
                    FetchCommands::Models => {
                        self.runtime
                            .block_on(async { fetch_models().await })
                            .change_context(AppError)?;
                    }
                }
            }
            Commands::Bench { subcommand } => {
                if cli.db_path.is_some() {
                    return Err(Report::new(AppError)
                        .attach("--db-path cannot be used with bench subcommands. Use 'bench tui <db_path>' instead"));
                }
                match subcommand {
                    BenchCommands::Run {
                        db_path,
                        model,
                        task,
                        csv,
                        artifact_dir,
                    } => {
                        if let Some(ref dir) = artifact_dir {
                            std::fs::create_dir_all(dir)
                                .change_context(AppError)
                                .attach("failed to create artifact directory")?;
                        }

                        // Build the bench plan from models × tasks.
                        let plan = nullslop_bench::orchestrator::build_plan(&model, &task)
                            .change_context(AppError)
                            .attach("invalid task glob pattern")?;
                        tracing::info!(pairs = plan.pairs.len(), "built bench plan");

                        let (core, services, actor_host) =
                            actor_wiring::create_core_with_actor_host(
                                &self.handle(),
                                llm_service,
                                provider_registry,
                                resolved_api_keys,
                                config_storage,
                                SessionStoreService::new(Arc::new(
                                    SqliteSessionStore::open_or_create(&db_path)
                                        .expect("failed to create bench session store"),
                                )),
                                UserPreferencesStorageService::new(Arc::new(
                                    FilesystemUserPreferencesStorage::default_path(),
                                )),
                                Some(csv),
                                Some(plan),
                                artifact_dir,
                            );
                        let paths = &services.paths;
                        load_prompt_templates(
                            &core.state,
                            &paths.prompts_dir(),
                            &paths.system_prompts_dir(),
                        );
                        load_compaction_prompt(
                            &core.state,
                            &paths.prompts_dir(),
                            &paths.system_prompts_dir(),
                        )?;
                        load_theme(&core.state, &paths.themes_dir(), &paths.system_themes_dir());

                        let mouse_selection = !matches!(std::env::var("NULLSLOP_MOUSE_SELECTION"), Ok(val) if val.eq_ignore_ascii_case("false") || val == "0");
                        let tui_config = nullslop_tui::config::TuiConfig::new(mouse_selection);
                        let mut ui_registry = nullslop_domain::AppUiRegistry::new();
                        nullslop_domain::register_all_ui_elements(&mut ui_registry);
                        let which_key = nullslop_tui::app::WhichKeyInstance::new(
                            nullslop_tui::keymap::init(),
                            nullslop_tui::Scope::Normal,
                        );

                        let runner = Runner::Tui(Box::new(nullslop_tui::TuiApp {
                            core,
                            services,
                            actor_host,
                            ui_registry,
                            events: nullslop_tui::MsgHandler::new(),
                            which_key,
                            suspend: nullslop_tui::suspend::Suspend::new(),
                            event_thread: None,
                            status: nullslop_tui::AppStatus::Starting,
                            selection: nullslop_tui::selection::SelectionState::Idle,
                            selectable_rects: Default::default(),
                            pending_clipboard: false,
                            config: tui_config,
                            sidebar: {
                                let mut s = nullslop_domain::feat::ui::sidebar::Sidebar::new();
                                nullslop_domain::feat::ui::sidebar::register_sections(&mut s);
                                s
                            },
                            preview_cache: nullslop_tui::app::PreviewCache::new(),
                            plugin_host: None,
                            welcome_subscriber: None,
                        }));
                        runner.run().change_context(AppError)?;
                    }
                    BenchCommands::Show { csv } => {
                        nullslop_bench::show::show_results(&csv).map_err(|e| {
                            error_stack::Report::new(AppError).attach(e.to_string())
                        })?;
                    }
                    BenchCommands::Compare { csv_a, csv_b } => {
                        nullslop_bench::compare::compare_results(&csv_a, &csv_b).map_err(|e| {
                            error_stack::Report::new(AppError).attach(e.to_string())
                        })?;
                    }
                    BenchCommands::Tui { db_path } => {
                        let session_store = SessionStoreService::new(Arc::new(
                            SqliteSessionStore::open_or_create(&db_path)
                                .expect("failed to create session store"),
                        ));
                        let (core, services, actor_host) =
                            actor_wiring::create_core_with_actor_host(
                                &self.handle(),
                                llm_service.clone(),
                                provider_registry.clone(),
                                resolved_api_keys.clone(),
                                config_storage.clone(),
                                session_store,
                                UserPreferencesStorageService::new(Arc::new(
                                    FilesystemUserPreferencesStorage::default_path(),
                                )),
                                None,
                                None,
                                None,
                            );
                        let paths = &services.paths;
                        load_prompt_templates(
                            &core.state,
                            &paths.prompts_dir(),
                            &paths.system_prompts_dir(),
                        );
                        load_compaction_prompt(
                            &core.state,
                            &paths.prompts_dir(),
                            &paths.system_prompts_dir(),
                        )?;
                        load_theme(&core.state, &paths.themes_dir(), &paths.system_themes_dir());

                        let mouse_selection = !matches!(std::env::var("NULLSLOP_MOUSE_SELECTION"), Ok(val) if val.eq_ignore_ascii_case("false") || val == "0");
                        let tui_config = nullslop_tui::config::TuiConfig::new(mouse_selection);
                        let mut ui_registry = nullslop_domain::AppUiRegistry::new();
                        nullslop_domain::register_all_ui_elements(&mut ui_registry);
                        let which_key = nullslop_tui::app::WhichKeyInstance::new(
                            nullslop_tui::keymap::init(),
                            nullslop_tui::Scope::Normal,
                        );

                        let runner = Runner::Tui(Box::new(nullslop_tui::TuiApp {
                            core,
                            services,
                            actor_host,
                            ui_registry,
                            events: nullslop_tui::MsgHandler::new(),
                            which_key,
                            suspend: nullslop_tui::suspend::Suspend::new(),
                            event_thread: None,
                            status: nullslop_tui::AppStatus::Starting,
                            selection: nullslop_tui::selection::SelectionState::Idle,
                            selectable_rects: Default::default(),
                            pending_clipboard: false,
                            config: tui_config,
                            sidebar: {
                                let mut s = nullslop_domain::feat::ui::sidebar::Sidebar::new();
                                nullslop_domain::feat::ui::sidebar::register_sections(&mut s);
                                s
                            },
                            preview_cache: nullslop_tui::app::PreviewCache::new(),
                            plugin_host: None,
                            welcome_subscriber: None,
                        }));
                        runner.run().change_context(AppError)?;
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new().expect("failed to create default App")
    }
}

/// Loads prompt templates from both user and system directories into the application state.
///
/// Called once after core creation. Failures are logged but not fatal —
/// an empty store is used when both directories are missing or unreadable.
fn load_prompt_templates(state: &State, user_dir: &Path, system_dir: &Path) {
    let store = nullslop_domain::PromptTemplateStore::load_from_dirs(user_dir, system_dir)
        .unwrap_or_else(|e| {
            tracing::warn!("failed to load prompt templates: {e:?}");
            nullslop_domain::PromptTemplateStore::new()
        });
    tracing::info!(count = store.len(), "loaded prompt templates");
    state.write().context.prompt_templates = store;
}

/// Loads the compaction system prompt from user or system prompts directory.
///
/// Searches the user prompts directory first (`~/.config/nullslop/prompts/_compaction.md`),
/// then the system prompts directory (`/usr/share/nullslop/prompts/_compaction.md`).
///
/// # Errors
///
/// Returns an error if the compaction prompt is missing from both directories
/// or cannot be read. This is a fatal error — the application cannot run without it.
fn load_compaction_prompt(
    state: &State,
    user_dir: &Path,
    system_dir: &Path,
) -> Result<(), Report<AppError>> {
    let prompt = nullslop_domain::common::system_resource::load_system_resource(
        "_compaction.md",
        user_dir,
        system_dir,
    )
    .change_context(AppError)
    .attach("failed to load compaction prompt")?;

    tracing::info!("loaded compaction prompt");
    state.write().context.compaction_prompt = prompt;
    Ok(())
}

/// Loads the theme from user preferences into the application state.
///
/// Searches the user themes directory first, then the system themes directory.
/// If the preferred theme cannot be loaded, falls back to the default theme.
/// Failures are logged but not fatal.
fn load_theme(state: &State, user_dir: &Path, system_dir: &Path) {
    let theme_name = {
        let guard = state.read();
        guard.frontend.preferences.theme_name.clone()
    };
    match nullslop_domain::feat::theme::resolve_theme(theme_name.as_deref(), user_dir, system_dir) {
        Ok(theme) => {
            tracing::info!(theme = ?theme_name, "loaded theme");
            state.write().frontend.theme = theme;
        }
        Err(e) => {
            tracing::warn!(err = ?e, "failed to load theme, using default");
        }
    }
}

/// Fetches model metadata from models.dev and saves it to the user's cache directory.
///
/// Makes an HTTP GET request to `https://models.dev/api.json`, validates the
/// response as JSON, and writes it to `~/.cache/nullslop/models.dev.json`.
///
/// # Errors
///
/// Returns an error if the HTTP request fails, the response is not valid JSON,
/// or the file cannot be written.
async fn fetch_models() -> Result<(), Report<AppError>> {
    use nullslop_domain::common::app_info::APP_NAME;

    tracing::info!(url = "https://models.dev/api.json", "fetching model metadata");

    let response = reqwest::get("https://models.dev/api.json")
        .await
        .change_context(AppError)
        .attach("failed to fetch models.dev API")?;

    if !response.status().is_success() {
        return Err(Report::new(AppError).attach(format!(
            "models.dev returned HTTP {}",
            response.status()
        )));
    }

    let body = response
        .text()
        .await
        .change_context(AppError)
        .attach("failed to read models.dev response")?;

    // Validate that the response is valid JSON and count providers/models.
    let (provider_count, model_count) = {
        let parsed: serde_json::Value = serde_json::from_str(&body)
            .change_context(AppError)
            .attach("models.dev response is not valid JSON")?;

        let mut provider_count = 0u32;
        let mut model_count = 0u32;
        if let serde_json::Value::Object(map) = &parsed {
            for (_provider_name, provider_data) in map {
                provider_count += 1;
                if let Some(models) = provider_data.get("models").and_then(|m| m.as_object()) {
                    model_count += models.len() as u32;
                }
            }
        }
        (provider_count, model_count)
    };

    // Write to user cache directory.
    let target_path = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(APP_NAME)
        .join("models.dev.json");

    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)
            .change_context(AppError)
            .attach("failed to create cache directory")?;
    }

    std::fs::write(&target_path, &body)
        .change_context(AppError)
        .attach("failed to write models.dev.json")?;

    println!(
        "Fetched {model_count} models from {provider_count} providers to {}",
        target_path.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use nullslop_domain::AppState;
    use std::path::PathBuf;

    use super::*;

    #[rstest::rstest]
    fn load_prompt_templates_sets_count() {
        // Given a temp directory with a template file.
        let dir = tempfile::tempdir().expect("temp dir");
        let template_content =
            "+++\nname = \"test\"\ndescription = \"Test template\"\n+++\nTest body.";
        std::fs::write(dir.path().join("test.md"), template_content).expect("write template");

        let state = State::new(AppState::default());

        // When loading prompt templates from the temp directory (user dir only).
        let empty = PathBuf::from("/nonexistent");
        load_prompt_templates(&state, dir.path(), &empty);

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

        // When loading prompt templates from the temp directory (user dir only).
        let empty = PathBuf::from("/nonexistent");
        load_prompt_templates(&state, dir.path(), &empty);

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
}
