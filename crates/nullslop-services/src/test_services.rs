//! Test utilities for constructing [Services] with fake implementations.
//!
//! [`TestServices`] provides a builder that creates a [`Services`] instance
//! with all fake/noop implementations, suitable for unit tests that need
//! a [`Services`] but don't test provider behavior.

use std::sync::Arc;

use error_stack::Report;
use kanal::Sender;
use nullslop_context::{DefaultStrategyDiscovery, StrategyDiscovery};
use nullslop_protocol::{AppMsg, SessionId};
use nullslop_providers::{
    ApiKeys, ApiKeysService, ConfigStorageService, InMemoryConfigStorage, LlmServiceFactoryService,
    ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};
use nullslop_session::{
    PersistedSession, SessionStore, SessionStoreError, SessionStoreService, SessionSummary,
};
use tokio::runtime::Handle;

use crate::Services;
use crate::actor_channel::ActorChannelService;
use crate::core_channel::{CoreChannelService, CoreNotification};
use crate::strategy_registry::StrategyRegistryService;

/// A no-op session store for tests.
///
/// All operations succeed with empty results. Suitable for tests that
/// need a [`Services`] but don't test session persistence.
#[derive(Debug)]
pub struct FakeSessionStore;

impl SessionStore for FakeSessionStore {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn save(&self, _session: &PersistedSession) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    fn load_summaries(
        &self,
    ) -> Result<Vec<(SessionId, SessionSummary, u64)>, Report<SessionStoreError>> {
        Ok(Vec::new())
    }

    fn load_full(
        &self,
        _byte_offset: u64,
    ) -> Result<Option<PersistedSession>, Report<SessionStoreError>> {
        Ok(None)
    }

    fn compact(&self) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }
}

/// A builder for constructing [Services] with fake implementations for tests.
///
/// All services default to empty/noop implementations. Use the builder methods
/// to customize specific services when needed.
///
/// Uses a leaked tokio runtime — acceptable for unit tests.
///
/// # Example
///
/// ```ignore
/// let services = TestServices::builder().build();
/// let state = AppState::default();
/// ```
pub struct TestServices {
    /// Provider configuration for the registry.
    providers: ProvidersConfig,
    /// Custom tokio runtime handle (if provided).
    handle: Option<Handle>,
    /// Custom actor channel sender (if provided).
    actor_channel_sender: Option<Sender<AppMsg>>,
    /// Custom core channel sender (if provided).
    core_channel_sender: Option<Sender<CoreNotification>>,
    /// Custom LLM service factory (if provided).
    llm_service: Option<LlmServiceFactoryService>,
    /// Custom session store service (if provided).
    session_store: Option<SessionStoreService>,
    /// Custom strategy discovery (if provided).
    strategy_discovery: Option<Arc<dyn StrategyDiscovery>>,
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
            core_channel_sender: None,
            llm_service: None,
            session_store: None,
            strategy_discovery: None,
        }
    }
}

impl TestServices {
    /// Create a new builder with defaults.
    #[must_use]
    pub fn builder() -> Self {
        Self::default()
    }

    /// Set the provider configuration.
    #[must_use]
    pub fn with_providers(mut self, config: ProvidersConfig) -> Self {
        self.providers = config;
        self
    }

    /// Set a custom tokio runtime handle.
    #[must_use]
    pub fn handle(mut self, handle: Handle) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Set a custom actor channel sender.
    #[must_use]
    pub fn actor_channel(mut self, sender: Sender<AppMsg>) -> Self {
        self.actor_channel_sender = Some(sender);
        self
    }

    /// Set a custom core channel sender.
    #[must_use]
    pub fn core_channel(mut self, sender: Sender<CoreNotification>) -> Self {
        self.core_channel_sender = Some(sender);
        self
    }

    /// Set a custom LLM service factory.
    #[must_use]
    pub fn llm_service(mut self, service: LlmServiceFactoryService) -> Self {
        self.llm_service = Some(service);
        self
    }

    /// Set a custom session store service.
    #[must_use]
    pub fn session_store(mut self, store: SessionStoreService) -> Self {
        self.session_store = Some(store);
        self
    }

    /// Set a custom strategy discovery.
    #[must_use]
    pub fn strategy_discovery(mut self, discovery: Arc<dyn StrategyDiscovery>) -> Self {
        self.strategy_discovery = Some(discovery);
        self
    }

    /// Build the [`Services`] instance.
    ///
    /// Leaks a tokio runtime if no custom handle is provided — acceptable for unit tests.
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

        let (actor_tx, _actor_rx) = kanal::unbounded::<AppMsg>();
        let (core_tx, _core_rx) = kanal::unbounded::<CoreNotification>();

        Services {
            handle,
            actor_channel: ActorChannelService::new(self.actor_channel_sender.unwrap_or(actor_tx)),
            core_channel: CoreChannelService::new(self.core_channel_sender.unwrap_or(core_tx)),
            llm_service: self.llm_service.unwrap_or_else(|| {
                LlmServiceFactoryService::new(Arc::new(
                    nullslop_providers::FakeLlmServiceFactory::new(vec![]),
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
            strategy_registry: match self.strategy_discovery {
                Some(d) => StrategyRegistryService::new(d),
                None => StrategyRegistryService::new(Arc::new(DefaultStrategyDiscovery)),
            },
        }
    }
}
