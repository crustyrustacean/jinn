//! Application-wide runtime services.
//!
//! This crate defines the [`Services`] container, which holds long-lived
//! runtime infrastructure that subsystems need access to. It is created
//! once during startup and shared throughout the application.

use std::sync::Arc;

use crate::feat::preferences_actor::{
    InMemoryUserPreferencesStorage, UserPreferencesStorageService,
};
pub use crate::feat::provider_infra;
use crate::feat::provider_infra::{
    ApiKeys, ApiKeysService, ConfigStorageService, InMemoryConfigStorage, LlmServiceFactoryService,
    ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};
use crate::feat::session::SessionStoreService;
use crate::protocol::AppMsg;
use tokio::runtime::Handle;

pub mod actor_channel;

#[cfg(test)]
mod actor_channel_tests;

pub mod test_services;

pub use actor_channel::ActorChannelService;

/// Runtime services shared across the application.
///
/// Holds references to all services, enabling dependency injection
/// and making it easy to swap implementations for testing.
///
/// Production code should construct this via struct initialization syntax
/// to get compiler-verified completeness.
///
/// Tests can use [`Services::new()`] which provides all-fake defaults,
/// or [`test_services::TestServices::builder()`] to customize specific services.
#[derive(Debug, Clone)]
pub struct Services {
    /// Application filesystem paths (configured once at init).
    pub paths: crate::common::app_paths::AppPaths,
    /// Async runtime handle for spawning background tasks.
    pub handle: Handle,
    /// Channel for sending commands/events into the actor system.
    pub actor_channel: ActorChannelService,
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
    /// User preferences storage for persisting `jinn.toml`.
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

        let temp_dir = Box::leak(Box::new(tempfile::TempDir::new().expect("test temp dir")));

        let (actor_tx, _actor_rx) = kanal::unbounded::<AppMsg>();
        Self {
            paths: crate::common::app_paths::AppPaths::new_in(temp_dir.path()),
            handle,
            actor_channel: ActorChannelService::new(actor_tx),
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
            user_preferences_storage: UserPreferencesStorageService::new(Arc::new(
                InMemoryUserPreferencesStorage::new(),
            )),
        }
    }
}
