//! Test services builder for unit tests.

use std::sync::Arc;

use crate::common::services::NoopPluginFire;
use crate::common::services::NoopPluginSyncCall;
use crate::feat::preferences_actor::{
    InMemoryUserPreferencesStorage, UserPreferencesStorageService,
};
use crate::feat::provider_infra::{
    FakeLlmServiceFactory, InMemoryConfigStorage, LlmServiceFactoryService, ProviderRegistry,
    ProviderRegistryService, ProvidersConfig,
};
use crate::common::services::ConfigStorageService;
use crate::feat::session::SessionStoreService;
use crate::feat::workflow::{PluginFire, PluginSyncCall};
use crate::protocol::AppMsg;

use super::{ActorChannelService, ApiKeys, ApiKeysService, Services};
use tokio::runtime::Handle;

/// Builder for constructing [`Services`] with custom overrides for tests.
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

    /// Build the `Services` with configured overrides.
    ///
    /// # Panics
    ///
    /// Panics if the tokio runtime fails to create (extremely unlikely in tests).
    #[must_use]
    #[expect(clippy::expect_used, reason = "test-only code, panics are acceptable")]
    pub fn build(self) -> Services {
        let handle = self.handle.unwrap_or_else(|| {
            let rt = Box::leak(Box::new(
                tokio::runtime::Runtime::new().expect("test runtime"),
            ));
            rt.handle().clone()
        });

        let temp_dir = Box::leak(Box::new(tempfile::TempDir::new().expect("test temp dir")));

        let (actor_tx, _actor_rx) = kanal::unbounded::<AppMsg>();
        Services {
            paths: self
                .paths
                .unwrap_or_else(|| crate::common::app_paths::AppPaths::new_in(temp_dir.path())),
            handle,
            actor_channel: ActorChannelService::new(self.actor_channel_sender.unwrap_or(actor_tx)),
            llm_service: self.llm_service.unwrap_or_else(|| {
                LlmServiceFactoryService::new(Arc::new(
                    FakeLlmServiceFactory::new(vec![]),
                ))
            }),
            provider_registry: ProviderRegistryService::new(
                ProviderRegistry::from_config(self.providers).expect("test registry"),
            ),
            api_keys: ApiKeysService::new(ApiKeys::new()),
            config_storage: ConfigStorageService::new(Arc::new(InMemoryConfigStorage::new())),
            session_store: self
                .session_store
                .unwrap_or_else(|| SessionStoreService::new(Arc::new(FakeSessionStore))),
            user_preferences_storage: UserPreferencesStorageService::new(Arc::new(
                InMemoryUserPreferencesStorage::new(),
            )),
            plugins: Arc::new(NoopPluginFire) as Arc<dyn PluginFire>,
            plugin_sync: Arc::new(NoopPluginSyncCall) as Arc<dyn PluginSyncCall>,
        }
    }
}

/// Fake session store for tests.
pub struct FakeSessionStore;

#[async_trait::async_trait]
impl crate::feat::session::SessionStore for FakeSessionStore {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn save(
        &self,
        _session: &crate::feat::session::ChatSessionState,
    ) -> Result<(), error_stack::Report<crate::feat::session::SessionStoreError>> {
        Ok(())
    }

    async fn load_summaries(
        &self,
    ) -> Result<Vec<crate::feat::session::SessionSummary>, error_stack::Report<crate::feat::session::SessionStoreError>> {
        Ok(Vec::new())
    }

    async fn load_session(
        &self,
        _session_id: &crate::protocol::SessionId,
    ) -> Result<Option<crate::feat::session::ChatSessionState>, error_stack::Report<crate::feat::session::SessionStoreError>> {
        Ok(None)
    }

    async fn delete(
        &self,
        _session_id: &crate::protocol::SessionId,
    ) -> Result<(), error_stack::Report<crate::feat::session::SessionStoreError>> {
        Ok(())
    }

    async fn fork(
        &self,
        _source_session_id: &crate::protocol::SessionId,
        _at_ordinal: usize,
    ) -> Result<crate::protocol::SessionId, error_stack::Report<crate::feat::session::SessionStoreError>> {
        Ok(crate::protocol::SessionId::new())
    }

    async fn set_archived(
        &self,
        _session_id: &crate::protocol::SessionId,
        _archived: bool,
    ) -> Result<(), error_stack::Report<crate::feat::session::SessionStoreError>> {
        Ok(())
    }

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<crate::feat::session::SessionSummary>, error_stack::Report<crate::feat::session::SessionStoreError>> {
        Ok(Vec::new())
    }
}
