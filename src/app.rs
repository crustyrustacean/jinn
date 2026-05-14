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
        use nullslop_cli::cli::{Commands, HeadlessCommands};

        // Load config from providers.toml (auto-creates on first run).
        let config_storage =
            ConfigStorageService::new(Arc::new(FilesystemConfigStorage::default_path()));
        // API keys are resolved by the env-init actor.
        let resolved_api_keys = ApiKeysService::new(ApiKeys::new());

        // Provider registry is populated by the provider-init actor.
        // Start with an empty registry.
        let empty_config = nullslop_domain::ProvidersConfig {
            providers: vec![],
            aliases: vec![],
            default_provider: None,
        };
        let provider_registry = ProviderRegistryService::new(
            ProviderRegistry::from_config(empty_config).change_context(AppError)?,
        );

        // Initial factory is the no-provider sentinel until actors resolve the real one.
        let llm_service = LlmServiceFactoryService::new(Arc::new(NoProvidersAvailableFactory));

        match cli.command.unwrap_or(Commands::Tui) {
            Commands::Tui => {
                ensure_persona_seed();
                let (core, services, actor_host) = actor_wiring::create_core_with_actor_host(
                    &self.handle(),
                    llm_service.clone(),
                    provider_registry.clone(),
                    resolved_api_keys.clone(),
                    config_storage.clone(),
                    UserPreferencesStorageService::new(Arc::new(
                        FilesystemUserPreferencesStorage::default_path(),
                    )),
                );
                ensure_prompt_example();
                load_prompt_templates(&core.state, &nullslop_domain::prompts_dir());

                // Resolve mouse selection config from environment.
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
                    event_task: None,
                    status: nullslop_tui::AppStatus::Starting,
                    tab_manager: nullslop_tui::render::init_tab_manager(),
                    selection: nullslop_tui::selection::SelectionState::Idle,
                    selectable_rects: Default::default(),
                    pending_clipboard: false,
                    config: tui_config,
                    sidebar: {
                        let mut s = nullslop_domain::feat::ui::sidebar::Sidebar::new();
                        nullslop_domain::feat::ui::sidebar::register_sections(&mut s);
                        s
                    },
                }));
                runner.run().change_context(AppError)?;
            }
            Commands::Headless { command, .. } => {
                ensure_persona_seed();
                let (core, _services, actor_host) = actor_wiring::create_core_with_actor_host(
                    &self.handle(),
                    llm_service.clone(),
                    provider_registry,
                    resolved_api_keys,
                    config_storage,
                    UserPreferencesStorageService::new(Arc::new(
                        FilesystemUserPreferencesStorage::default_path(),
                    )),
                );
                ensure_prompt_example();
                load_prompt_templates(&core.state, &nullslop_domain::prompts_dir());
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
        }

        Ok(())
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new().expect("failed to create default App")
    }
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

/// Ensures the personas directory exists and contains the seed persona.
///
/// Called once after core creation. Failures are logged but not fatal —
/// the seed persona is a convenience, not a requirement.
fn ensure_persona_seed() {
    if let Err(e) = nullslop_domain::ensure_personas_dir_with_seed() {
        tracing::warn!("failed to ensure persona seed: {e:?}");
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
    use nullslop_domain::AppState;

    use super::*;

    fn test_cli(command: Option<Commands>) -> Cli {
        Cli {
            verbosity: Verbosity::new(0, 0),
            log_dir: None,
            command,
        }
    }

    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(15))]
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
}
