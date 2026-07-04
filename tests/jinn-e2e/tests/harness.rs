//! Shared e2e test harness: builds a real actor system (in-memory deps + a
//! scripted fake LLM factory) and wraps it in a full [`TuiApp`] with rendering
//! disabled, ready for command- or keystroke-driven scenarios.
//!
//! The returned [`TuiApp`] lives on the caller's thread — it is `!Send`
//! (Lua state + UI trait objects), so it must never cross threads. Because
//! each scenario runs in its own process (see [`runner`]), graceful shutdown
//! is handled by process exit; no owned runtime is needed.

use std::path::Path;
use std::sync::Arc;

use jinn::actor_wiring::{ActorSystemBuilder, ActorSystemBuilderArgs};
use jinn_domain::AppPaths;
use jinn_domain::{
    ApiKeys, ApiKeysService, AppStateStorageService, ConfigStorageService, FakeLlmServiceFactory,
    InMemoryAppStateStorage, InMemoryConfigStorage, InMemoryUserPreferencesStorage,
    LlmServiceFactoryService, ProviderRegistry, ProviderRegistryService, ProvidersConfig,
    SessionStoreService, SqliteSessionStore, UserPreferencesStorageService,
};
use jinn_tui::TuiApp;
use jinn_tui::launch::launch_for_test;

/// Builds a real actor system with in-memory deps inside `temp_root`, then
/// wraps it in a full [`TuiApp`] (rendering disabled) driven by the fake LLM
/// factory.
///
/// `fake_factory` is handed to the service wrapper (erased) **and** returned
/// typed via the caller's clone, so the world can keep the queueing API
/// (`push_scripted_response`) reachable.
///
/// Must be called from inside a tokio context (cucumber's runtime), and the
/// returned `TuiApp` must stay on the calling thread.
pub async fn build_tuiapp_in_temp(
    temp_root: &Path,
    fake_factory: Arc<FakeLlmServiceFactory>,
) -> TuiApp {
    let config_storage = ConfigStorageService::new(Arc::new(InMemoryConfigStorage::new()));
    let resolved_api_keys = ApiKeysService::new(ApiKeys::new());
    let test_config = ProvidersConfig {
        providers: vec![jinn_domain::ProviderEntry {
            name: "test".to_owned(),
            backend: "sample".to_owned(), // backend is irrelevant — the override wins.
            models: vec!["test".to_owned()],
            base_url: None,
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        aliases: vec![],
        default_provider: None,
    };
    let provider_registry = ProviderRegistryService::new(
        ProviderRegistry::from_config(test_config)
            .expect("test config is valid")
            // Inject the fake as the factory for the `test/test` provider id so the
            // per-request factory path in the LLM actor resolves to the scripted
            // fake instead of erroring on the empty production registry.
            .with_factory_override(fake_factory.clone()),
    );
    let llm_service = LlmServiceFactoryService::new(fake_factory);
    let user_preferences_storage = {
        let svc =
            UserPreferencesStorageService::new(Arc::new(InMemoryUserPreferencesStorage::new()));
        svc.reload().expect("test prefs storage initial reload");
        svc
    };
    let paths = AppPaths::new_in(temp_root);
    let session_store = SessionStoreService::new(Arc::new(
        SqliteSessionStore::new_in(&paths.sessions_dir())
            .await
            .expect("store"),
    ));
    let app_state_storage = AppStateStorageService::new(Arc::new(InMemoryAppStateStorage::new()));
    app_state_storage
        .reload()
        .expect("test state storage initial reload");

    let handle = tokio::runtime::Handle::current();

    let (core, services, sync_plugins, _discord_rx) =
        ActorSystemBuilder::new(ActorSystemBuilderArgs {
            handle,
            llm_service,
            provider_registry,
            api_keys: resolved_api_keys,
            config_storage,
            session_store,
            user_preferences_storage,
            app_state_storage,
            paths,
        })
        .build()
        .await;

    launch_for_test(core, services, sync_plugins)
}

/// Copies a repo-bundled attachable plugin (`res/plugins/attachable/<name>/`)
/// into the temp config tree so `attach(name)` resolves a real `init.lua`
/// during the actor-system load.
///
/// Must be called **before** [`build_tuiapp_in_temp`] so the loader (which
/// runs at actor-system startup) discovers the plugin.
pub fn copy_plugin_to_temp(temp_root: &Path, plugin_name: &str) {
    let manifest_dir = option_env!("CARGO_MANIFEST_DIR").unwrap_or(".").to_string();
    let repo_root = std::path::Path::new(&manifest_dir)
        .ancestors()
        .nth(2)
        .unwrap_or_else(|| panic!("could not resolve workspace root from {manifest_dir:?}"));
    let src = repo_root.join("res/plugins/attachable").join(plugin_name);
    let dst_dir = temp_root
        .join("config/jinn/plugins/attachable")
        .join(plugin_name);
    std::fs::create_dir_all(&dst_dir).unwrap_or_else(|e| panic!("mkdir {dst_dir:?}: {e}"));
    // Copy the whole plugin dir (init.lua + any siblings).
    let read = std::fs::read_dir(&src).unwrap_or_else(|e| panic!("read plugin dir {src:?}: {e}"));
    for entry in read {
        let entry = entry.expect("dir entry");
        let file_type = entry.file_type().expect("file type");
        if file_type.is_file() {
            let file_name = entry.file_name();
            std::fs::copy(entry.path(), dst_dir.join(&file_name))
                .unwrap_or_else(|e| panic!("copy {:?} -> {dst_dir:?}: {e}", entry.path()));
        }
    }
}

/// Seeds a prompt template into the temp tree's user prompts dir.
///
/// Writes `<name>.md` with TOML frontmatter (`name`, `description`) + body,
/// matching the format `prompt_scan_actor` scans. Must be called **before**
/// [`build_tuiapp_in_temp`] so the scan at actor-system startup picks it up.
pub fn seed_prompt_template(temp_root: &Path, name: &str, body: &str) {
    let dir = temp_root.join("config").join("jinn").join("prompts");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("mkdir {dir:?}: {e}"));
    let content = format!("+++\nname = \"{name}\"\ndescription = \"\"\n+++\n{body}");
    std::fs::write(dir.join(format!("{name}.md")), content)
        .unwrap_or_else(|e| panic!("write prompt {name}: {e}"));
}

/// Readiness predicate for [`World::wait_until`]: resolves once the named
/// prompt template is present in the active session's discovered store, i.e.
/// `prompt_scan_actor` has finished its startup scan for this session's cwd.
pub fn prompt_template_ready_predicate(
    name: &str,
) -> impl Fn(&jinn_domain::AppState) -> Option<()> + '_ {
    move |s| {
        s.session
            .active_session()
            .discovered_prompt_templates()
            .find_by_name(name)
            .is_some()
            .then_some(())
    }
}
