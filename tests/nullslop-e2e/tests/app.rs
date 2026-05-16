//! Cucumber `World` wrapping a full application with production actor wiring.
//!
//! The [`AppWorld`] creates a complete application using the same
//! `actor_wiring::create_core_with_actor_host` function that production uses,
//! but with fake services so no real backends are hit. All 16 actors spawn,
//! init sequences run, and the system-ready signal fires.
//!
//! This is the standard e2e world for all future feature files.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use cucumber::World;
use nullslop::actor_wiring;
use nullslop_domain::ApiKeys;
use nullslop_domain::ApiKeysService;
use nullslop_domain::AppState;
use nullslop_domain::AppUiRegistry;
use nullslop_domain::ConfigStorageService;
use nullslop_domain::FakeLlmServiceFactory;
use nullslop_domain::InMemoryConfigStorage;
use nullslop_domain::InMemoryUserPreferencesStorage;
use nullslop_domain::LlmServiceFactoryService;
use nullslop_domain::ProviderRegistry;
use nullslop_domain::ProviderRegistryService;
use nullslop_domain::ProvidersConfig;
use nullslop_domain::SessionLoadRequested;
use nullslop_domain::StateReadGuard;
use nullslop_domain::UserPreferencesStorageService;
use nullslop_tui::AppStatus;
use nullslop_tui::MsgHandler;
use nullslop_tui::Scope;
use nullslop_tui::TuiApp;
use nullslop_tui::app::WhichKeyInstance;
use nullslop_tui::config::TuiConfig;
use nullslop_tui::render;
use nullslop_tui::selection::SelectionState;
use nullslop_tui::suspend::Suspend;

/// Cucumber world wrapping a full application with production actor wiring.
///
/// Created fresh for each scenario. Provides the full actor system
/// (all 16 actors) backed by fake services.
#[derive(World)]
#[world(init = Self::new_app_world)]
pub struct AppWorld {
    /// The full TUI application under test (no terminal backend connected).
    pub app: TuiApp,
    /// Tokio runtime handle.
    #[allow(dead_code)]
    handle: tokio::runtime::Handle,
    /// Temp directory holding all test filesystem paths. Cleaned up on drop.
    #[allow(dead_code)]
    temp_dir: tempfile::TempDir,
    /// Pre-reload CWD captured during "saved and reloaded" step.
    cwd_before_reload: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for AppWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppWorld")
            .field("state", &self.app.core.state)
            .finish_non_exhaustive()
    }
}

impl AppWorld {
    /// Creates a new world with the full production actor wiring and fake services.
    ///
    /// Spawns a dedicated tokio runtime on a separate thread so that
    /// `create_core_with_actor_host` can call `blocking_recv` (which panics
    /// inside an existing runtime context like the cucumber test runner).
    /// The `TuiApp` is then constructed on the calling thread from the
    /// cross-thread-safe results.
    fn new_app_world() -> Self {
        // Run setup on a separate thread to avoid
        // "Cannot block the current thread from within a runtime".
        // Only the core/services/actor_host cross the thread boundary
        // (TuiApp is !Send due to trait objects).
        let (handle_tx, handle_rx) = std::sync::mpsc::channel();
        let temp_dir = tempfile::TempDir::new().expect("test temp dir");
        let temp_dir_path = temp_dir.path().to_path_buf();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("test runtime");
            let handle = rt.handle().clone();

            // Build fake services — same pattern as production App::dispatch
            // but with all fake implementations.
            let paths = nullslop_domain::AppPaths::new_in(&temp_dir_path);
            let config_storage = ConfigStorageService::new(Arc::new(InMemoryConfigStorage::new()));
            let resolved_api_keys = ApiKeysService::new(ApiKeys::new());
            let empty_config = ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            };
            let provider_registry = ProviderRegistryService::new(
                ProviderRegistry::from_config(empty_config).expect("empty config is valid"),
            );
            let llm_service =
                LlmServiceFactoryService::new(Arc::new(FakeLlmServiceFactory::new(vec![])));
            let user_preferences_storage =
                UserPreferencesStorageService::new(Arc::new(InMemoryUserPreferencesStorage::new()));
            let session_store = nullslop_domain::SessionStoreService::new(Arc::new(
                nullslop_domain::SqliteSessionStore::new_in(paths.sessions_dir()),
            ));

            // Call production wiring — spawns all 16 actors.
            let (core, services, actor_host) = actor_wiring::create_core_with_actor_host(
                &handle,
                llm_service,
                provider_registry,
                resolved_api_keys,
                config_storage,
                session_store,
                user_preferences_storage,
            );

            // Leak the runtime so it lives for the test duration.
            let _ = Box::leak(Box::new(rt));

            handle_tx
                .send((handle, core, services, actor_host))
                .expect("send results");
        });

        let (handle, core, services, actor_host) = handle_rx.recv().expect("receive setup results");

        // Build TuiApp following the production App::dispatch pattern.
        let mut ui_registry = AppUiRegistry::new();
        nullslop_domain::register_all_ui_elements(&mut ui_registry);

        let app = TuiApp {
            core,
            services,
            actor_host,
            ui_registry,
            events: MsgHandler::new(),
            which_key: WhichKeyInstance::new(nullslop_tui::keymap::init(), Scope::Normal),
            suspend: Suspend::new(),
            event_task: None,
            status: AppStatus::Starting,
            tab_manager: render::init_tab_manager(),
            selection: SelectionState::Idle,
            selectable_rects: Default::default(),
            pending_clipboard: false,
            config: TuiConfig::default(),
            sidebar: {
                let mut s = nullslop_domain::feat::ui::sidebar::Sidebar::new();
                nullslop_domain::feat::ui::sidebar::register_sections(&mut s);
                s
            },
        };

        Self {
            app,
            handle,
            temp_dir,
            cwd_before_reload: None,
        }
    }

    /// Polls `AppState` at 10ms intervals until `predicate` returns `true`
    /// or the 5-second timeout expires.
    ///
    /// Use in `When` steps that trigger async actor work, so `Then` steps
    /// can assert synchronously.
    pub async fn wait_until(&self, predicate: impl Fn(&AppState) -> bool) {
        let state = self.app.core.state.clone();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if predicate(&state.read()) {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Sends a keystroke to the app.
    pub fn press_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let event = crossterm::event::Event::Key(KeyEvent::new(code, modifiers));
        self.app.handle_msg(nullslop_tui::msg::Msg::Input(event));
    }

    /// Routes an intent through the app.
    pub fn route_intent(&mut self, intent: nullslop_domain::Intent) {
        self.app.route_intent(intent);
    }

    /// Submits a command to the core's message channel.
    #[allow(dead_code)]
    pub fn submit_command(&self, cmd: nullslop_domain::Command) {
        self.app.core.submit_command(cmd);
    }

    /// Returns a read guard to the application state.
    pub fn state(&self) -> StateReadGuard<'_> {
        self.app.core.state.read()
    }

    /// Runs graceful coordinated shutdown of the actor system.
    #[allow(dead_code)]
    pub fn graceful_shutdown(&mut self) {
        nullslop_domain::coordinated_shutdown(
            self.app.actor_host.backend(),
            &self.app.core.state,
            &self.handle,
            nullslop_domain::SHUTDOWN_TIMEOUT,
        );
    }
}

// ---------------------------------------------------------------------------
// Step definitions
// ---------------------------------------------------------------------------

/// Parses a human-readable key name into a [`KeyCode`].
fn parse_key_code(name: &str) -> KeyCode {
    // Match special key names case-insensitively.
    match name.to_lowercase().as_str() {
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "tab" => KeyCode::Tab,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "delete" => KeyCode::Delete,
        "space" => KeyCode::Char(' '),
        _ => {
            // Single characters preserve case (G vs g are different keys).
            if name.len() == 1 {
                KeyCode::Char(name.chars().next().expect("single char"))
            } else {
                panic!("unknown key: {name}")
            }
        }
    }
}

/// Parses a human-readable modifier name into [`KeyModifiers`].
fn parse_modifier(name: &str) -> KeyModifiers {
    match name.to_lowercase().as_str() {
        "shift" => KeyModifiers::SHIFT,
        "ctrl" | "control" => KeyModifiers::CONTROL,
        "alt" => KeyModifiers::ALT,
        _ => panic!("unknown modifier: {name}"),
    }
}

/// Parses a human-readable mode name into [`nullslop_domain::Mode`].
fn parse_mode(name: &str) -> nullslop_domain::Mode {
    match name.to_lowercase().as_str() {
        "normal" => nullslop_domain::Mode::Normal,
        "input" => nullslop_domain::Mode::Input,
        "picker" => nullslop_domain::Mode::Picker,
        _ => panic!("unknown mode: {name}"),
    }
}

// --- Given steps ---

/// World is already initialised with a fresh AppWorld.
#[cucumber::given(expr = "a fresh app")]
fn given_a_fresh_app(_world: &mut AppWorld) {}

/// Sets the app's mode by pushing the appropriate scope onto the scope stack.
#[cucumber::given(expr = "the app is in {word} mode")]
fn given_app_in_mode(world: &mut AppWorld, mode: String) {
    let scope = match parse_mode(&mode) {
        nullslop_domain::Mode::Normal => Scope::Normal,
        nullslop_domain::Mode::Input => {
            let mut state = world.app.core.state.write();
            state
                .frontend
                .scope_stack
                .push(nullslop_domain::common::app_state::FocusScope::Input);
            drop(state);
            Scope::Input
        }
        nullslop_domain::Mode::Picker => Scope::Picker,
    };
    world.app.which_key.set_scope(scope);
}

/// Pre-fills the active chat input buffer with the given text.
#[cucumber::given(expr = "the input buffer contains {string}")]
fn given_input_buffer_contains(world: &mut AppWorld, text: String) {
    world
        .app
        .core
        .state
        .write()
        .active_chat_input_mut()
        .replace_all(text.to_owned());
}

/// Sets the active provider to a dummy value so message submission works.
#[cucumber::given(expr = "the active provider is set")]
fn given_active_provider_set(world: &mut AppWorld) {
    world
        .app
        .core
        .state
        .write()
        .active_session_mut()
        .set_model("test".to_owned());
}

// --- When steps ---

/// Simulates the user pressing a single key (no modifiers).
#[cucumber::when(expr = "the user presses {word}")]
fn when_user_presses_key(world: &mut AppWorld, key: String) {
    let code = parse_key_code(&key);
    world.press_key(code, KeyModifiers::NONE);
}

/// Simulates the user pressing a key with a modifier.
#[cucumber::when(expr = "the user presses {word} with {word}")]
fn when_user_presses_key_with_mod(world: &mut AppWorld, key: String, modifier: String) {
    let code = parse_key_code(&key);
    let mods = parse_modifier(&modifier);
    world.press_key(code, mods);
}

/// Routes a ToggleWhichKey command directly.
#[cucumber::when(expr = "the app routes the ToggleWhichKey command")]
fn when_routes_toggle_which_key(world: &mut AppWorld) {
    world.route_intent(nullslop_domain::Intent::ToggleWhichkey);
}

/// Runs a headless script through the keymap pipeline.
#[cucumber::when(expr = "I run the headless script {string}")]
fn when_run_headless_script(world: &mut AppWorld, script: String) {
    run_headless_script(world, &script);
}

/// Shared implementation for running a headless script.
fn run_headless_script(world: &mut AppWorld, content: &str) {
    let leader = nullslop_domain::KeyEvent {
        key: nullslop_domain::Key::Char('\\'),
        modifiers: nullslop_domain::Modifiers::none(),
    };
    let lines: Vec<Vec<nullslop_domain::KeyEvent>> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| ratatui_which_key::parse_key_sequence(line, &leader))
        .collect();

    for keys in lines {
        for key in keys {
            let state_read = world.app.core.state.read();
            let scope = nullslop_tui::app::scope_for_focus(
                state_read.frontend.scope_stack.current(),
                state_read.frontend.active_tab,
            );
            drop(state_read);
            world.app.which_key.set_scope(scope);
            if let Some(intent) = world.app.which_key.handle_key(key) {
                world.route_intent(intent);
            }
        }
    }
}

// --- Then steps ---

/// Asserts the application's current mode matches the expected value.
#[cucumber::then(expr = "the mode should be {word}")]
fn then_mode_should_be(world: &mut AppWorld, mode: String) {
    let expected = parse_mode(&mode);
    let actual = world
        .app
        .core
        .state
        .read()
        .frontend
        .scope_stack
        .current()
        .mode();
    assert_eq!(
        actual, expected,
        "expected mode {expected:?}, got {actual:?}"
    );
}

/// Asserts the application has requested to quit.
#[cucumber::then(expr = "the app should quit")]
fn then_app_should_quit(world: &mut AppWorld) {
    let should_quit = world.app.core.state.read().frontend.should_quit;
    assert!(
        should_quit,
        "expected app to quit, but should_quit is false"
    );
}

/// Asserts the application has NOT requested to quit.
#[cucumber::then(expr = "the app should not quit")]
fn then_app_should_not_quit(world: &mut AppWorld) {
    let should_quit = world.app.core.state.read().frontend.should_quit;
    assert!(
        !should_quit,
        "expected app to not quit, but should_quit is true"
    );
}

/// Asserts the active chat input buffer is empty.
#[cucumber::then(expr = "the input buffer should be empty")]
fn then_input_buffer_empty(world: &mut AppWorld) {
    let text = world
        .app
        .core
        .state
        .read()
        .active_chat_input()
        .text()
        .to_owned();
    assert!(
        text.is_empty(),
        "expected empty input buffer, got: {text:?}"
    );
}

/// Asserts the active chat input buffer matches the expected text.
#[cucumber::then(expr = "the input buffer should be {string}")]
fn then_input_buffer_should_be(world: &mut AppWorld, expected: String) {
    let actual = world
        .app
        .core
        .state
        .read()
        .active_chat_input()
        .text()
        .to_owned();
    let expected = expected.replace("\\n", "\n").replace("\\t", "\t");
    assert_eq!(actual, expected, "input buffer mismatch");
}

/// Asserts the active session's chat history contains the expected number of entries.
/// Waits up to 5 seconds for the count to match.
#[cucumber::then(expr = "the chat history should contain {int} entry")]
async fn then_chat_history_count(world: &mut AppWorld, count: u64) {
    let expected = count as usize;
    world
        .wait_until(|state| state.active_session().history().len() >= expected)
        .await;
    let actual = world.state().active_session().history().len();
    assert_eq!(
        actual, expected,
        "expected {count} history entries, got {actual}"
    );
}

/// Asserts the active session's chat history contains at least the expected number of entries.
/// Waits up to 5 seconds for the count to match.
#[cucumber::then(expr = "the chat history should contain at least {int} entry")]
async fn then_chat_history_at_least_count(world: &mut AppWorld, count: u64) {
    let expected = count as usize;
    world
        .wait_until(|state| state.active_session().history().len() >= expected)
        .await;
    let actual = world.state().active_session().history().len();
    assert!(
        actual >= expected,
        "expected at least {count} history entries, got {actual}"
    );
}

/// Asserts the which-key popup is active.
#[cucumber::then(expr = "which-key should be active")]
fn then_which_key_active(world: &mut AppWorld) {
    assert!(
        world.app.which_key.active,
        "expected which-key to be active"
    );
}

/// Asserts the which-key popup is inactive.
#[cucumber::then(expr = "which-key should be inactive")]
fn then_which_key_inactive(world: &mut AppWorld) {
    assert!(
        !world.app.which_key.active,
        "expected which-key to be inactive"
    );
}

// --- Chat scroll step definitions ---

/// Asserts the cursor is on the last entry in the chat history.
/// Waits up to 5 seconds for the history to have at least one entry.
#[cucumber::then(expr = "the cursor should be on the last entry")]
async fn then_cursor_on_last_entry(world: &mut AppWorld) {
    world
        .wait_until(|state| {
            let history_len = state.active_session().history().len();
            history_len > 0
                && state.active_session().selected_entry_index() == Some(history_len - 1)
        })
        .await;
    let state = world.state();
    let history_len = state.active_session().history().len();
    let cursor = state.active_session().selected_entry_index();
    assert_eq!(
        cursor,
        Some(history_len - 1),
        "expected cursor on last entry ({})",
        history_len - 1
    );
}

/// Asserts the cursor is on a specific entry by index.
#[cucumber::then(expr = "the cursor should be on entry {int}")]
fn then_cursor_on_entry(world: &mut AppWorld, index: u64) {
    let state = world.state();
    let cursor = state.active_session().selected_entry_index();
    assert_eq!(
        cursor,
        Some(index as usize),
        "expected cursor on entry {index}, got {:?}",
        cursor
    );
}

/// Asserts the scroll is at the bottom (auto-scroll position).
#[cucumber::then(expr = "the scroll should be at the bottom")]
fn then_scroll_at_bottom(world: &mut AppWorld) {
    let state = world.state();
    assert!(
        state.active_session().is_at_bottom(),
        "expected scroll at bottom"
    );
}

// --- Prompt template expansion step definitions ---

/// Injects a prompt template into the app state's template store.
#[cucumber::given(expr = "a prompt template {string} with body {string}")]
fn given_prompt_template(world: &mut AppWorld, name: String, body: String) {
    let mut state = world.app.core.state.write();
    let mut templates = state.context.prompt_templates.templates().to_vec();
    templates.push(nullslop_domain::PromptTemplate {
        name,
        description: String::new(),
        body,
    });
    state.context.prompt_templates = nullslop_domain::PromptTemplateStore::from_vec(templates);
}

/// Asserts the last User entry has the expected display and expanded text.
#[cucumber::then(expr = "the last user entry has display {string} and expanded {string}")]
fn then_last_user_entry_display_expanded(world: &mut AppWorld, display: String, expanded: String) {
    let state = world.state();
    let history = state.active_session().history();
    let user_entries: Vec<_> = history
        .iter()
        .rev()
        .filter(|e| matches!(&e.kind, nullslop_domain::ChatEntryKind::User { .. }))
        .collect();
    let last = user_entries
        .first()
        .expect("expected at least one user entry");
    match &last.kind {
        nullslop_domain::ChatEntryKind::User {
            display: actual_display,
            expanded: actual_expanded,
        } => {
            assert_eq!(actual_display, &display, "display text mismatch");
            assert_eq!(actual_expanded, &expanded, "expanded text mismatch");
        }
        _ => panic!("expected User entry, got {:?}", last.kind),
    }
}

// --- Session CWD step definitions ---

/// Asserts the active session's CWD is not empty.
#[cucumber::then(expr = "the session CWD should not be empty")]
fn then_session_cwd_not_empty(world: &mut AppWorld) {
    let state = world.state();
    let cwd = state.active_session().cwd();
    assert!(
        !cwd.as_os_str().is_empty(),
        "expected non-empty CWD, got: {:?}",
        cwd
    );
}

/// Saves the active session, captures its CWD, then triggers a reload.
#[cucumber::when(expr = "the session is saved and reloaded")]
async fn when_session_saved_and_reloaded(world: &mut AppWorld) {
    // Wait for any pending actor work to complete (e.g., session save after
    // message enqueue). The session actor saves asynchronously, so we wait
    // for the history to stabilize.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Capture CWD before reload.
    let (session_id, cwd_before) = {
        let state = world.state();
        let session = state.active_session();
        (
            session.session_id().clone(),
            session.cwd().to_owned(),
        )
    };

    // Trigger reload by sending SessionLoadRequested.
    world.submit_command(nullslop_domain::Command::SessionLoadRequested(
        nullslop_domain::SessionLoadRequested {
            session_id: session_id.clone(),
        },
    ));

    // Wait for the load to complete (session_loading transitions to false).
    world
        .wait_until(|state| !state.session.session_loading)
        .await;

    // Wait a bit more for the async cwd check to complete.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Store the pre-reload CWD for comparison.
    world.cwd_before_reload = Some(cwd_before);
}

/// Asserts the session CWD is the same as before the reload.
#[cucumber::then(expr = "the session CWD should be preserved")]
fn then_session_cwd_preserved(world: &mut AppWorld) {
    let state = world.state();
    let cwd_after = state.active_session().cwd();
    let cwd_before = world
        .cwd_before_reload
        .as_ref()
        .expect("no pre-reload CWD stored");
    assert_eq!(
        cwd_after, cwd_before,
        "CWD not preserved across reload"
    );
}

/// Sets the active session's CWD to a non-existent path.
#[cucumber::given(expr = "the session CWD is set to a non-existent path")]
fn given_session_cwd_nonexistent(world: &mut AppWorld) {
    world
        .app
        .core
        .state
        .write()
        .active_session_mut()
        .set_cwd(std::path::PathBuf::from("/nonexistent/test/path/xyz"));
}

/// Asserts a warning about the missing CWD appears in chat history.
#[cucumber::then(expr = "a warning about the missing CWD should appear")]
async fn then_warning_about_missing_cwd(world: &mut AppWorld) {
    world
        .wait_until(|state| {
            state
                .active_session()
                .history()
                .iter()
                .any(|e| {
                    matches!(&e.kind, nullslop_domain::ChatEntryKind::System(t) if t.contains("Warning: working directory"))
                })
        })
        .await;
    let state = world.state();
    let found = state
        .active_session()
        .history()
        .iter()
        .any(|e| {
            matches!(&e.kind, nullslop_domain::ChatEntryKind::System(t) if t.contains("Warning: working directory"))
        });
    assert!(found, "expected a warning about missing CWD in chat history");
}

/// Asserts the session CWD has fallen back to the global default.
#[cucumber::then(expr = "the session CWD should fall back to the global CWD")]
fn then_session_cwd_fallback(world: &mut AppWorld) {
    let state = world.state();
    let actual = state.active_session().cwd();
    let expected = &state.session.default_cwd;
    assert_eq!(
        actual, expected,
        "expected CWD to fall back to global CWD"
    );
}
