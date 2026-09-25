//! LLM service abstraction - streaming chat completions.
//!
//! Defines the [`LlmService`] trait for streaming LLM responses and
//! [`LlmServiceFactory`] for creating per-call service instances.
//! Includes an `OpenRouter` implementation, a sample provider for UI testing,
//! and a generic factory that supports any `LLMBackend` via config.

mod api_keys;
mod api_keys_service;
mod config;
mod config_storage;
mod generic_factory;
mod model_cache;
mod models_dev;
mod provider_id;
mod registry;

mod registry_service;
#[cfg(test)]
mod registry_service_tests;
#[cfg(test)]
mod registry_tests;
mod resolved_provider;
mod retry;
mod service_wrapper;
#[cfg(test)]
mod template_validation_tests;

pub use api_keys::ApiKeys;
pub use api_keys_service::ApiKeysService;
pub use config::{
    AliasEntry, AlloyStrategy, ConfigError, InitProvidersError, InitProvidersOutcome,
    ModelInfoEntry, ProviderEntry, ProvidersConfig, config_path, create_default_config,
    init_default_providers_to, load_config, save_config,
};
pub use config_storage::{
    ConfigStorage, ConfigStorageService, FilesystemConfigStorage, InMemoryConfigStorage,
};
pub use generic_factory::GenericLlmServiceFactory;
pub use jinn_provider::{
    ChatStream, FakeLlmServiceFactory, InputModalities, LlmService, LlmServiceError,
    LlmServiceFactory, Modality, ModelInfo, NoProvidersAvailableFactory, SampleLlmServiceFactory,
    ScriptedResponse, StopReason, StreamEvent, TOOL_LOOP_TRIGGER, ToolStream,
};
pub use model_cache::{ModelCache, ModelCacheError, cache_path};
pub use models_dev::ModelsDevData;
pub use provider_id::ProviderId;
pub use registry::ProviderRegistry;
pub use registry_service::ProviderRegistryService;
pub use resolved_provider::ResolvedProvider;
pub use retry::request_retry_to_provider_config;
pub use service_wrapper::LlmServiceFactoryService;
