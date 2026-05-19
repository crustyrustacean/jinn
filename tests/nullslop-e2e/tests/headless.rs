//! Cucumber `World` wrapping a headless application.
//!
//! The [`HeadlessWorld`] creates a complete application using the same
//! `actor_wiring::create_core_with_actor_host` function that production uses,
//! but with fake services and a temp directory so no real backends are hit.
//! The resulting core is wrapped in a [`HeadlessApp`] instead of a `TuiApp`.

use std::sync::Arc;

use cucumber::World;
use nullslop::actor_wiring;
use nullslop::headless::HeadlessApp;
use nullslop_domain::ApiKeys;
use nullslop_domain::ApiKeysService;
use nullslop_domain::ConfigStorageService;
use nullslop_domain::InMemoryConfigStorage;
use nullslop_domain::InMemoryUserPreferencesStorage;
use nullslop_domain::LlmServiceFactoryService;
use nullslop_domain::NoProvidersAvailableFactory;
use nullslop_domain::ProviderRegistry;
use nullslop_domain::ProviderRegistryService;
use nullslop_domain::ProvidersConfig;
use nullslop_domain::SessionStoreService;
use nullslop_domain::SqliteSessionStore;
use nullslop_domain::UserPreferencesStorageService;

/// Cucumber world wrapping a headless application with production actor wiring.
///
/// Created fresh for each scenario. Provides the full actor system
/// backed by fake services, wrapped in a [`HeadlessApp`].
#[derive(World)]
#[world(init = Self::new_headless_world)]
pub struct HeadlessWorld {
    /// The headless application under test.
    /// Wrapped in a helper struct to provide a `Debug` impl
    /// since `HeadlessApp` does not implement `Debug`.
    headless: Option<HeadlessApp>,
    /// Tokio runtime handle.
    #[allow(dead_code)]
    handle: tokio::runtime::Handle,
    /// Temp directory holding all test filesystem paths. Cleaned up on drop.
    #[allow(dead_code)]
    temp_dir: tempfile::TempDir,
    /// Whether the last operation succeeded.
    last_ok: Option<bool>,
}

impl std::fmt::Debug for HeadlessWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeadlessWorld")
            .field("last_ok", &self.last_ok)
            .finish_non_exhaustive()
    }
}

impl HeadlessWorld {
    /// Creates a new world with the full production actor wiring and fake services.
    fn new_headless_world() -> Self {
        let temp_dir = tempfile::TempDir::new().expect("test temp dir");
        let (headless, handle) = Self::build_headless(temp_dir.path());
        Self {
            headless: Some(headless),
            handle,
            temp_dir,
            last_ok: None,
        }
    }

    /// Builds a [`HeadlessApp`] with fake services at the given temp directory.
    fn build_headless(temp_path: &std::path::Path) -> (HeadlessApp, tokio::runtime::Handle) {
        // Run setup on a separate thread to avoid
        // "Cannot block the current thread from within a runtime".
        let (tx, rx) = std::sync::mpsc::channel();
        let temp_dir_path = temp_path.to_path_buf();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("test runtime");
            let handle = rt.handle().clone();

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
            let llm_service = LlmServiceFactoryService::new(Arc::new(NoProvidersAvailableFactory));
            let user_preferences_storage =
                UserPreferencesStorageService::new(Arc::new(InMemoryUserPreferencesStorage::new()));
            let session_store = SessionStoreService::new(Arc::new(
                SqliteSessionStore::new_in(&paths.sessions_dir()).expect("store"),
            ));

            // Call production wiring — spawns all 16 actors.
            let (core, _services, actor_host) = actor_wiring::create_core_with_actor_host(
                &handle,
                llm_service,
                provider_registry,
                resolved_api_keys,
                config_storage,
                session_store,
                user_preferences_storage,
            );

            // Intentionally leaked: each world gets a fresh tokio runtime.
            let _ = Box::leak(Box::new(rt));

            tx.send((core, actor_host, handle))
                .expect("send setup results");
        });

        let (core, actor_host, handle) = rx.recv().expect("receive setup results");
        let headless = HeadlessApp::new(core, actor_host, handle.clone());

        (headless, handle)
    }
}

// ---------------------------------------------------------------------------
// Step definitions
// ---------------------------------------------------------------------------

/// World is already initialised with a fresh HeadlessWorld.
#[cucumber::given(expr = "a headless app")]
fn given_a_headless_app(_world: &mut HeadlessWorld) {}

/// Runs a script containing the given text.
#[cucumber::when(expr = r#"a script containing {string} is run"#)]
fn when_script_containing_is_run(world: &mut HeadlessWorld, content: String) {
    let mut headless = world.headless.take().expect("headless app");
    let dir = tempfile::tempdir().expect("temp dir for script");
    let script_path = dir.path().join("test.script");
    std::fs::write(&script_path, content).expect("write script");
    let file = std::fs::File::open(&script_path).expect("open script file");
    world.last_ok = Some(headless.run_script(file).is_ok());
    world.headless = Some(headless);
}

/// Simulates the missing-file error path from `App::dispatch`.
///
/// The production code opens the file first via `std::fs::File::open`,
/// then passes the handle to `run_script`. If `File::open` fails, dispatch
/// returns `Err` before reaching `run_script`. We replicate that here.
#[cucumber::when(expr = "a script is run from a missing file")]
fn when_script_from_missing_file(world: &mut HeadlessWorld) {
    let file_result = std::fs::File::open("/no/such/file.script");
    match file_result {
        Ok(file) => {
            let mut headless = world.headless.take().expect("headless app");
            world.last_ok = Some(headless.run_script(file).is_ok());
            world.headless = Some(headless);
        }
        Err(_) => {
            // File::open failed — same as the production dispatch path.
            world.last_ok = Some(false);
        }
    }
}

/// Asserts the script completed without error.
#[cucumber::then(expr = "the script should complete successfully")]
fn then_script_completes_successfully(world: &mut HeadlessWorld) {
    let ok = world.last_ok.take().expect("script result");
    assert!(ok, "expected script to succeed, but it failed");
}

/// Asserts the script failed.
#[cucumber::then(expr = "the script should fail")]
fn then_script_should_fail(world: &mut HeadlessWorld) {
    let ok = world.last_ok.take().expect("script result");
    assert!(!ok, "expected script to fail, but it succeeded");
}
