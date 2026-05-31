#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

//! Test utilities for constructing [Services] with fake implementations.
//!
//! [`TestServices`] provides a builder that creates a [`Services`] instance
//! with all fake/noop implementations, suitable for unit tests that need
//! a [`Services`] but don't test provider behavior.

use std::sync::Arc;

use crate::feat::preferences_actor::{
    InMemoryUserPreferencesStorage, UserPreferencesStorageService,
};
use crate::feat::provider_infra::{
    ApiKeys, ApiKeysService, ConfigStorageService, InMemoryConfigStorage, LlmServiceFactoryService,
    ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::{SessionStore, SessionStoreError, SessionStoreService, SessionSummary};
use crate::protocol::{AppMsg, SessionId};
use async_trait::async_trait;
use error_stack::Report;
use kanal::Sender;
use tokio::runtime::Handle;

use super::Services;
use super::actor_channel::ActorChannelService;

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

    async fn load_judge_sessions_for_origin(
        &self,
        _origin_session_id: &SessionId,
    ) -> Result<Vec<crate::feat::session::chat_session::ChatSessionState>, Report<SessionStoreError>>
    {
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
    actor_channel_sender: Option<Sender<AppMsg>>,
    /// Custom LLM service factory (if provided).
    llm_service: Option<LlmServiceFactoryService>,
    /// Custom session store service (if provided).
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

    /// Set custom app paths.
    #[must_use]
    pub fn paths(mut self, paths: crate::common::app_paths::AppPaths) -> Self {
        self.paths = Some(paths);
        self
    }

    /// Build the [`Services`] instance.
    ///
    /// Leaks a tokio runtime if no custom handle is provided - acceptable for unit tests.
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
                    crate::feat::provider_infra::FakeLlmServiceFactory::new(vec![]),
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
        }
    }
}
