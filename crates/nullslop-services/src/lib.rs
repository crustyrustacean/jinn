//! Application-wide runtime services.
//!
//! This crate defines the [`Services`] container, which holds long-lived
//! runtime infrastructure that subsystems need access to. It is created
//! once during startup and shared throughout the application.

use std::sync::Arc;

use nullslop_actor_host::ActorHostService;
pub use nullslop_providers as providers;
use nullslop_providers::{
    ApiKeysService, ConfigStorageService, LlmServiceFactoryService, ProviderRegistryService,
};
use nullslop_session::SessionStoreService;
use tokio::runtime::Handle;

pub mod strategy_registry;
pub mod test_services;

use crate::strategy_registry::StrategyRegistryService;

/// Runtime services shared across the application.
///
/// Holds references to all services, enabling dependency injection
/// and making it easy to swap implementations for testing.
#[derive(Debug, Clone)]
pub struct Services {
    /// Async runtime handle for spawning background tasks.
    handle: Handle,
    /// Actor host service.
    actor_host: ActorHostService,
    /// LLM service factory for creating streaming chat instances.
    llm_service: LlmServiceFactoryService,
    /// Provider registry for looking up and validating provider configs.
    provider_registry: ProviderRegistryService,
    /// Resolved API keys for provider availability checks and factory creation.
    api_keys: ApiKeysService,
    /// Config storage for persisting provider configuration.
    config_storage: ConfigStorageService,
    /// Session store for persisting chat session data.
    session_store: SessionStoreService,
    /// Strategy discovery for listing available prompt assembly strategies.
    strategy_registry: StrategyRegistryService,
}

impl Services {
    /// Creates a new `Services` with the given components.
    #[must_use]
    #[expect(clippy::too_many_arguments, reason = "service container needs all dependencies")]
    pub fn new(
        handle: Handle,
        actor_host: Arc<dyn nullslop_actor_host::ActorHost>,
        llm_service: LlmServiceFactoryService,
        provider_registry: ProviderRegistryService,
        api_keys: ApiKeysService,
        config_storage: ConfigStorageService,
        session_store: SessionStoreService,
        strategy_registry: StrategyRegistryService,
    ) -> Self {
        Self {
            handle,
            actor_host: ActorHostService::new(actor_host),
            llm_service,
            provider_registry,
            api_keys,
            config_storage,
            session_store,
            strategy_registry,
        }
    }

    /// Returns a reference to the async runtime handle.
    #[must_use]
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// Returns a reference to the actor host service.
    #[must_use]
    pub fn actor_host(&self) -> &ActorHostService {
        &self.actor_host
    }

    /// Returns a reference to the LLM service factory.
    #[must_use]
    pub fn llm_service(&self) -> &LlmServiceFactoryService {
        &self.llm_service
    }

    /// Returns a reference to the provider registry service.
    #[must_use]
    pub fn provider_registry(&self) -> &ProviderRegistryService {
        &self.provider_registry
    }

    /// Returns a reference to the resolved API keys service.
    #[must_use]
    pub fn api_keys(&self) -> &ApiKeysService {
        &self.api_keys
    }

    /// Returns a reference to the config storage service.
    #[must_use]
    pub fn config_storage(&self) -> &ConfigStorageService {
        &self.config_storage
    }

    /// Returns a reference to the session store service.
    #[must_use]
    pub fn session_store(&self) -> &SessionStoreService {
        &self.session_store
    }

    /// Returns a reference to the strategy registry service.
    #[must_use]
    pub fn strategy_registry(&self) -> &StrategyRegistryService {
        &self.strategy_registry
    }
}
