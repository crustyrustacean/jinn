#![expect(clippy::expect_used, reason = "test infrastructure initialization")]
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use error_stack::Report;
use tokio::runtime::{Handle, Runtime};

use crate::common::services::{NoopPluginFire, NoopPluginSyncCall, NoopSessionPluginRegistry};
use crate::feat::plugin_dispatch::{
    PluginFire, PluginFireService, PluginSyncCall, PluginSyncCallService,
};
use crate::feat::plugin_system::{SessionPluginRegistry, SessionPluginRegistryService};
use crate::feat::preferences_actor::{
    AppStateStorageService, InMemoryAppStateStorage, InMemoryUserPreferencesStorage,
    UserPreferencesStorageService,
};
use crate::feat::provider_infra::{
    ApiKeys, ApiKeysService, ConfigStorageService, FakeLlmServiceFactory, InMemoryConfigStorage,
    LlmServiceFactoryService, ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::{SessionStore, SessionStoreError, SessionStoreService, SessionSummary};
use crate::protocol::{AppMsg, SessionId};

use super::Services;
use super::actor_channel::ActorChannelService;
/// Single shared tokio runtime for the entire test binary.
///
/// Initializes exactly once via `LazyLock`. Without this, every
/// `Services::new()` / `TestServices::build()` call leaked a fresh
/// `Runtime` via `Box::leak`, which (across 3000+ parallel tests)
/// exhausted the FD limit (`EMFILE`).
///
/// The `Runtime` itself is intentionally leaked via `Box::leak` at
/// static-init time; it lives for the lifetime of the test binary.
/// The `Handle` is cheaply cloneable and shared by all tests.
static TEST_RUNTIME: LazyLock<Handle> = LazyLock::new(|| {
    let rt = Box::leak(Box::new(Runtime::new().expect("shared test runtime")));
    rt.handle().clone()
});

/// Returns a clone of the shared test runtime handle.
///
/// # Panics
///
/// Panics if the underlying tokio runtime fails to create (extremely
/// unlikely in tests).
pub(crate) fn shared_test_handle() -> Handle {
    TEST_RUNTIME.clone()
}

/// A no-op session store for tests.
///
/// All operations succeed with empty results. Suitable for tests that
/// need a [`Services`] but don't test session persistence.
#[derive(Debug)]
pub struct FakeSessionStore;

#[async_trait]
impl SessionStore for FakeSessionStore {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn save(&self, _session: &ChatSessionState) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn load_summaries(&self) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        Ok(Vec::new())
    }

    async fn load_session(
        &self,
        _session_id: &SessionId,
    ) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
        Ok(None)
    }

    async fn delete(&self, _session_id: &SessionId) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn fork(
        &self,
        _source_session_id: &SessionId,
        _at_ordinal: usize,
    ) -> Result<SessionId, Report<SessionStoreError>> {
        Ok(SessionId::new())
    }

    async fn set_archived(
        &self,
        _session_id: &SessionId,
        _archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<SessionSummary>, Report<SessionStoreError>> {
        Ok(Vec::new())
    }
}

/// A builder for constructing [Services] with fake implementations for tests.
///
/// All services default to empty/noop implementations. Use the builder methods
/// to customize specific services when needed.
///
/// Uses a leaked tokio runtime - acceptable for unit tests.
///
/// # Example
///
/// See the tests in this crate for full usage patterns.
pub struct TestServices {
    /// Provider configuration for the registry.
    providers: ProvidersConfig,
    /// Custom tokio runtime handle (if provided).
    handle: Option<Handle>,
    /// Custom actor channel sender (if provided).
    actor_channel_sender: Option<kanal::Sender<AppMsg>>,
    /// Custom LLM service factory (if provided).
    llm_service: Option<LlmServiceFactoryService>,
    /// Custom session store (if provided).
    session_store: Option<SessionStoreService>,
    /// Custom app paths (if provided).
    paths: Option<crate::common::app_paths::AppPaths>,
}

impl Default for TestServices {
    fn default() -> Self {
        Self {
            providers: ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            },
            handle: None,
            actor_channel_sender: None,
            llm_service: None,
            session_store: None,
            paths: None,
        }
    }
}

impl TestServices {
    /// Create a new builder with defaults.
    #[must_use]
    pub fn builder() -> Self {
        Self::default()
    }

    /// Set the providers config.
    #[must_use]
    pub fn providers(mut self, providers: ProvidersConfig) -> Self {
        self.providers = providers;
        self
    }

    /// Alias for [`providers`](Self::providers) for backward compat.
    #[must_use]
    pub fn with_providers(self, providers: ProvidersConfig) -> Self {
        self.providers(providers)
    }

    /// Set a custom runtime handle.
    #[must_use]
    pub fn handle(mut self, handle: Handle) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Set a custom actor channel sender.
    #[must_use]
    pub fn actor_channel_sender(mut self, sender: kanal::Sender<AppMsg>) -> Self {
        self.actor_channel_sender = Some(sender);
        self
    }

    /// Set a custom LLM service factory.
    #[must_use]
    pub fn llm_service(mut self, service: LlmServiceFactoryService) -> Self {
        self.llm_service = Some(service);
        self
    }

    /// Set a custom session store.
    #[must_use]
    pub fn session_store(mut self, store: SessionStoreService) -> Self {
        self.session_store = Some(store);
        self
    }

    /// Set custom app paths.
    #[must_use]
    pub fn paths(mut self, paths: crate::common::app_paths::AppPaths) -> Self {
        self.paths = Some(paths);
        self
    }

    /// Build the [`Services`] instance.
    ///
    /// Uses the shared process-wide test runtime if no custom handle is provided.
    ///
    /// # Panics
    ///
    /// Panics if the tokio runtime fails to create (extremely unlikely in tests).
    #[must_use]
    #[expect(clippy::expect_used, reason = "test-only code, panics are acceptable")]
    pub fn build(self) -> Services {
        let handle = self.handle.unwrap_or_else(shared_test_handle);

        let (paths, tempdir) = if let Some(p) = self.paths {
            (p, None)
        } else {
            let td = Arc::new(tempfile::TempDir::new().expect("test temp dir"));
            (
                crate::common::app_paths::AppPaths::new_in(td.path()),
                Some(td),
            )
        };

        // Keep a live drainer so the sender doesn't return ReceiveClosed.
        let (actor_tx, actor_rx) = kanal::unbounded::<AppMsg>();
        handle.spawn(async move {
            let rx = actor_rx.to_async();
            while rx.recv().await.is_ok() {}
        });
        Services {
            paths,
            handle,
            actor_channel: ActorChannelService::new(self.actor_channel_sender.unwrap_or(actor_tx)),
            llm_service: self.llm_service.unwrap_or_else(|| {
                LlmServiceFactoryService::new(Arc::new(FakeLlmServiceFactory::new(vec![])))
            }),
            provider_registry: ProviderRegistryService::new(
                ProviderRegistry::from_config(self.providers).expect("test registry"),
            ),
            api_keys: ApiKeysService::new(ApiKeys::new()),
            config_storage: ConfigStorageService::new(Arc::new(InMemoryConfigStorage::new())),
            session_store: self
                .session_store
                .unwrap_or_else(|| SessionStoreService::new(Arc::new(FakeSessionStore))),
            user_preferences_storage: {
                let svc = UserPreferencesStorageService::new(Arc::new(
                    InMemoryUserPreferencesStorage::new(),
                ));
                // Populate the cache so test code that calls .read() works.
                // InMemoryUserPreferencesStorage returns Ok(default) when empty.
                svc.reload().expect("test prefs storage initial reload");
                svc
            },
            app_state_storage: {
                let svc = AppStateStorageService::new(Arc::new(
                    InMemoryAppStateStorage::new(),
                ));
                svc.reload().expect("test app state storage initial reload");
                svc
            },
            plugins: PluginFireService::new(Arc::new(NoopPluginFire) as Arc<dyn PluginFire>),
            plugin_sync: PluginSyncCallService::new(
                Arc::new(NoopPluginSyncCall) as Arc<dyn PluginSyncCall>
            ),
            session_plugin_registry: SessionPluginRegistryService::new(Arc::new(
                NoopSessionPluginRegistry,
            )
                as Arc<dyn SessionPluginRegistry>),
            tempdir,
        }
    }
}
