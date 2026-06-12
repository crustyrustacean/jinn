//! Application-wide runtime services.
//!
//! This crate defines the [`Services`] container, which holds long-lived
//! runtime infrastructure that subsystems need access to. It is created
//! once during startup and shared throughout the application.

use std::sync::Arc;

use derive_more::Debug;
use kameo::actor::Spawn;

use crate::feat::plugin_dispatch::{
    PluginFire, PluginFireError, PluginSyncCall, PluginSyncCallError,
};
use crate::feat::plugin_system::SessionRegistryId;
use crate::feat::preferences_actor::{
    AppStateStorageService, InMemoryAppStateStorage, InMemoryUserPreferencesStorage,
    UserPreferencesStorageService,
};

pub use crate::feat::provider_infra;
use crate::feat::provider_infra::{
    ApiKeys, ApiKeysService, ConfigStorageService, InMemoryConfigStorage, LlmServiceFactoryService,
    ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};
use crate::feat::session::SessionStoreService;
use crate::protocol::AppMsg;
use error_stack::Report;
use tokio::runtime::Handle;

pub mod actor_channel;

pub mod bus_service;

pub mod test_services;

pub use actor_channel::ActorChannelService;
pub use bus_service::BusService;

#[cfg(test)]
pub use bus_service::{BusAudit, RecordedMessage};

/// Runtime services shared across the application.
///
/// Holds references to all services, enabling dependency injection
/// and making it easy to swap implementations for testing.
///
/// Production code should construct this via struct initialization syntax
/// to get compiler-verified completeness.
///
/// Tests can use [`Services::new()`] which provides all-fake defaults,
#[derive(Clone, Debug)]
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
    /// App state storage for persisting `state.toml`.
    pub app_state_storage: AppStateStorageService,
    /// Plugin system async handle (fire-and-forget + collect).
    pub plugins: crate::feat::plugin_dispatch::PluginFireService,
    /// Plugin system sync handle (blocking hook calls from actors).
    pub plugin_sync: crate::feat::plugin_dispatch::PluginSyncCallService,
    /// Per-session plugin registry (manages isolated Lua states for attached plugins).
    pub session_plugin_registry: crate::feat::plugin_system::SessionPluginRegistryService,
    /// Test-only owned temp directory. `None` in production.
    ///
    /// Held here so the dir outlives the [`AppPaths`] that points at it
    /// and is cleaned up when the last `Services` clone is dropped.
    /// Production code passes `None` because [`AppPaths::default`] resolves
    /// real user dirs.
    #[debug(skip)]
    pub tempdir: Option<Arc<tempfile::TempDir>>,

    /// Kameo message bus for type-based pub/sub routing.
    #[debug(skip)]
    pub bus: bus_service::BusService,

    /// Kanal closure bridge from sync TUI to async bus.
    #[debug(skip)]
    pub bridge: crate::common::bridge::Bridge,
}

impl Services {
    /// Creates a new `Services` with all fake/noop implementations.
    ///
    /// Suitable for unit tests that need a `Services` but don't test
    /// specific behavior. Shares a single process-wide tokio runtime
    /// across all tests to avoid FD exhaustion under parallel execution.
    ///
    /// # Panics
    ///
    /// Panics if the tokio runtime fails to create (extremely unlikely).
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "test-only defaults, panics are acceptable"
    )]
    pub async fn new_fake() -> Self {
        let handle = test_services::shared_test_handle();

        let tempdir = Arc::new(tempfile::TempDir::new().expect("test temp dir"));

        // Keep a live drainer so the sender doesn't return ReceiveClosed.
        // Without this, send_command silently fails in tests.
        let (actor_tx, actor_rx) = kanal::unbounded::<AppMsg>();
        handle.spawn(async move {
            let rx = actor_rx.to_async();
            while rx.recv().await.is_ok() {}
        });
        let bus = {
            let bus_actor = kameo_actors::message_bus::MessageBus::new(
                kameo_actors::DeliveryStrategy::BestEffort,
            );
            let bus_ref = kameo_actors::message_bus::MessageBus::spawn(bus_actor);
            bus_service::BusService::new(bus_ref)
        };
        let bridge = crate::common::bridge::Bridge::new(bus.actor_ref().clone());

        Self {
            paths: crate::common::app_paths::AppPaths::new_in(tempdir.path()),
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
            user_preferences_storage: {
                let svc = UserPreferencesStorageService::new(Arc::new(
                    InMemoryUserPreferencesStorage::new(),
                ));
                svc.reload().expect("test prefs storage initial reload");
                svc
            },
            app_state_storage: {
                let svc = AppStateStorageService::new(Arc::new(InMemoryAppStateStorage::new()));
                svc.reload().expect("test app state storage initial reload");
                svc
            },
            plugins: crate::feat::plugin_dispatch::PluginFireService::new(std::sync::Arc::new(
                NoopPluginFire,
            )
                as std::sync::Arc<dyn PluginFire>),
            plugin_sync: crate::feat::plugin_dispatch::PluginSyncCallService::new(
                std::sync::Arc::new(NoopPluginSyncCall) as std::sync::Arc<dyn PluginSyncCall>,
            ),
            session_plugin_registry: crate::feat::plugin_system::SessionPluginRegistryService::new(
                std::sync::Arc::new(NoopSessionPluginRegistry)
                    as std::sync::Arc<dyn crate::feat::plugin_system::SessionPluginRegistry>,
            ),
            tempdir: Some(tempdir),
            bus,
            bridge,
        }
    }

    /// Construct a fake Services with a pre-built bus (e.g. BusService::new_recording()).
    #[cfg(test)]
    pub async fn new_fake_with_bus(bus: bus_service::BusService) -> Self {
        let handle = test_services::shared_test_handle();
        let tempdir = Arc::new(tempfile::TempDir::new().expect("test temp dir"));

        // Keep a live drainer so the sender doesn't return ReceiveClosed.
        let (actor_tx, actor_rx) = kanal::unbounded::<AppMsg>();
        handle.spawn(async move {
            let rx = actor_rx.to_async();
            while rx.recv().await.is_ok() {}
        });
        let bridge = crate::common::bridge::Bridge::new_for_test();

        Self {
            paths: crate::common::app_paths::AppPaths::new_in(tempdir.path()),
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
            user_preferences_storage: {
                let svc = UserPreferencesStorageService::new(Arc::new(
                    InMemoryUserPreferencesStorage::new(),
                ));
                svc.reload().expect("test prefs storage initial reload");
                svc
            },
            app_state_storage: {
                let svc = AppStateStorageService::new(Arc::new(InMemoryAppStateStorage::new()));
                svc.reload().expect("test app state storage initial reload");
                svc
            },
            plugins: crate::feat::plugin_dispatch::PluginFireService::new(std::sync::Arc::new(
                NoopPluginFire,
            )
                as std::sync::Arc<dyn PluginFire>),
            plugin_sync: crate::feat::plugin_dispatch::PluginSyncCallService::new(
                std::sync::Arc::new(NoopPluginSyncCall) as std::sync::Arc<dyn PluginSyncCall>,
            ),
            session_plugin_registry: crate::feat::plugin_system::SessionPluginRegistryService::new(
                std::sync::Arc::new(NoopSessionPluginRegistry)
                    as std::sync::Arc<dyn crate::feat::plugin_system::SessionPluginRegistry>,
            ),
            tempdir: Some(tempdir),
            bus,
            bridge,
        }
    }
}

// ── Noop plugin implementations for test defaults ─────────────────────

/// Noop [`PluginFire`] for test defaults.
#[derive(Debug, Clone)]
pub struct NoopPluginFire;

/// Noop [`PluginSyncCall`] for test defaults.
#[derive(Debug, Clone)]
pub struct NoopPluginSyncCall;

/// Noop [`PluginFire`] for test defaults.
#[async_trait::async_trait]
impl PluginFire for NoopPluginFire {
    async fn fire_async_json(
        &self,
        hook: &str,
        _ctx: &serde_json::Value,
    ) -> Result<(), Report<PluginFireError>> {
        tracing::debug!(hook, "noop plugin fire");
        Ok(())
    }
    async fn fire_async_collect_json(
        &self,
        hook: &str,
        _ctx: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, Report<PluginFireError>> {
        tracing::debug!(hook, "noop plugin collect");
        Ok(vec![])
    }

    async fn fire_async_for_session_json(
        &self,
        _session: SessionRegistryId,
        hook: &str,
        _ctx: &serde_json::Value,
    ) -> Result<(), Report<PluginFireError>> {
        tracing::debug!(hook, "noop plugin fire for session");
        Ok(())
    }

    async fn fire_async_collect_for_session_json(
        &self,
        _session: SessionRegistryId,
        hook: &str,
        _ctx: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, Report<PluginFireError>> {
        tracing::debug!(hook, "noop plugin collect for session");
        Ok(vec![])
    }

    fn name(&self) -> &'static str {
        "NoopPluginFire"
    }
}

/// Noop [`PluginSyncCall`] for test defaults.
impl PluginSyncCall for NoopPluginSyncCall {
    fn call_hooks_json(
        &self,
        hook: &str,
        _ctx: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, Report<PluginSyncCallError>> {
        tracing::debug!(hook, "noop plugin sync");
        Ok(vec![])
    }

    fn call_hooks_for_session_json(
        &self,
        _session: SessionRegistryId,
        hook: &str,
        _ctx: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, Report<PluginSyncCallError>> {
        tracing::debug!(hook, "noop plugin sync for session");
        Ok(vec![])
    }
    fn name(&self) -> &'static str {
        "NoopPluginSyncCall"
    }
}

/// No-op implementation of [`SessionPluginRegistry`] for tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSessionPluginRegistry;

#[async_trait::async_trait]
impl crate::feat::plugin_system::SessionPluginRegistry for NoopSessionPluginRegistry {
    async fn create_session_registry(
        &self,
        _plugin_names: Vec<String>,
    ) -> Result<
        crate::feat::plugin_system::SessionRegistryId,
        Report<crate::feat::plugin_system::SessionPluginRegistryError>,
    > {
        Ok(crate::feat::plugin_system::SessionRegistryId::new())
    }

    async fn destroy_session_registry(
        &self,
        _registry_id: crate::feat::plugin_system::SessionRegistryId,
    ) -> Result<(), Report<crate::feat::plugin_system::SessionPluginRegistryError>> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "NoopSessionPluginRegistry"
    }
}
