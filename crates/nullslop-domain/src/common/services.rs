//! Application-wide runtime services.
//!
//! This crate defines the [`Services`] container, which holds long-lived
//! runtime infrastructure that subsystems need access to. It is created
//! once during startup and shared throughout the application.

use std::sync::Arc;

use crate::feat::context::DefaultStrategyDiscovery;
pub use crate::feat::provider_infra;
use crate::feat::provider_infra::{
    ApiKeys, ApiKeysService, ConfigStorageService, InMemoryConfigStorage, LlmServiceFactoryService,
    ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};
use crate::feat::preferences_actor::{InMemoryUserPreferencesStorage, UserPreferencesStorageService};
use crate::feat::session::SessionStoreService;
use crate::protocol::AppMsg;
use tokio::runtime::Handle;

pub mod actor_channel;
pub mod core_channel;
pub mod strategy_registry;
pub mod test_services;

pub use actor_channel::ActorChannelService;
pub use core_channel::{CoreChannelService, CoreNotification};

use strategy_registry::StrategyRegistryService;

/// Runtime services shared across the application.
///
/// Holds references to all services, enabling dependency injection
/// and making it easy to swap implementations for testing.
///
/// Production code should construct this via struct initialization syntax
/// to get compiler-verified completeness:
///
/// ```ignore
/// let services = Services {
///     handle: handle.clone(),
///     actor_channel,
///     core_channel,
///     llm_service,
///     provider_registry,
///     api_keys,
///     config_storage,
///     session_store,
///     strategy_registry,
/// };
/// ```
///
/// Tests can use [`Services::new()`] which provides all-fake defaults,
/// or [`test_services::TestServices::builder()`] to customize specific services.
#[derive(Debug, Clone)]
pub struct Services {
    /// Async runtime handle for spawning background tasks.
    pub handle: Handle,
    /// Channel for sending commands/events into the actor system.
    pub actor_channel: ActorChannelService,
    /// Channel for receiving lifecycle notifications from the actor system.
    pub core_channel: CoreChannelService,
    /// LLM service factory for creating streaming chat instances.
    pub llm_service: LlmServiceFactoryService,
    /// Provider registry for looking up and validating provider configs.
    pub provider_registry: ProviderRegistryService,
    /// Resolved API keys for provider availability checks and factory creation.
    pub api_keys: ApiKeysService,
    /// Config storage for persisting provider configuration.
    pub config_storage: ConfigStorageService,
    /// Session store for persisting chat session data.
    pub session_store: SessionStoreService,
    /// Strategy discovery for listing available prompt assembly strategies.
    pub strategy_registry: StrategyRegistryService,
    /// User preferences storage for persisting `nullslop.toml`.
    pub user_preferences_storage: UserPreferencesStorageService,
}

impl Default for Services {
    fn default() -> Self {
        Self::new()
    }
}

impl Services {
    /// Creates a new `Services` with all fake/noop implementations.
    ///
    /// Suitable for unit tests that need a `Services` but don't test
    /// specific service behavior. Leaks a tokio runtime — acceptable for tests.
    ///
    /// # Panics
    ///
    /// Panics if the tokio runtime fails to create (extremely unlikely).
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "test-only defaults, panics are acceptable"
    )]
    pub fn new() -> Self {
        let rt = Box::leak(Box::new(
            tokio::runtime::Runtime::new().expect("test runtime"),
        ));
        let handle = rt.handle().clone();

        let (actor_tx, _actor_rx) = kanal::unbounded::<AppMsg>();
        let (core_tx, _core_rx) = kanal::unbounded::<CoreNotification>();

        Self {
            handle,
            actor_channel: ActorChannelService::new(actor_tx),
            core_channel: CoreChannelService::new(core_tx),
            llm_service: LlmServiceFactoryService::new(Arc::new(
                crate::feat::provider_infra::FakeLlmServiceFactory::new(vec![]),
            )),
            provider_registry: ProviderRegistryService::new(
                ProviderRegistry::from_config(ProvidersConfig {
                    providers: vec![],
                    aliases: vec![],
                    default_provider: None,
                })
                .expect("empty config is valid"),
            ),
            api_keys: ApiKeysService::new(ApiKeys::new()),
            config_storage: ConfigStorageService::new(Arc::new(InMemoryConfigStorage::new())),
            session_store: SessionStoreService::new(Arc::new(test_services::FakeSessionStore)),
            strategy_registry: StrategyRegistryService::new(Arc::new(DefaultStrategyDiscovery)),
            user_preferences_storage: UserPreferencesStorageService::new(Arc::new(
                InMemoryUserPreferencesStorage::new(),
            )),
        }
    }
}
