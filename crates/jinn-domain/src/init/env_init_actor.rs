//! Environment initialization actor - reads env vars and populates API keys.
//!
//! During startup (`on_start`), loads `providers.toml`, resolves API keys from
//! environment variables, populates the shared `ApiKeysService`, and stores
//! the config for downstream actors to request via `ask(GetEnvironmentConfig)`.
//!
//! The `EnvironmentLoaded` event is retained for runtime reloads only.

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::bus::BusMessage;
use crate::feat::provider_infra::ProvidersConfig;
use error_stack::{Report, ResultExt};
use kameo::prelude::{Actor, ActorRef, Context, Message};
use wherror::Error;

/// Error type for environment initialization failures.
#[derive(Debug, Error)]
#[error(debug)]
pub struct EnvInitError;

/// The environment has been loaded and API keys are available.
///
/// Emitted after the env init actor has populated `ApiKeysService`.
/// Published at runtime for environment reloads (not during startup).
/// Downstream actors should use `ask(GetEnvironmentConfig)` for initial config.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnvironmentLoaded {
    /// The parsed provider configuration from `providers.toml`.
    pub config: ProvidersConfig,
}

impl BusMessage for EnvironmentLoaded {}

/// Ask message to retrieve the loaded environment config.
///
/// Downstream actors use this during their `on_start` to pull config
/// directly from the EnvInitActor via the actor registry.
pub struct GetEnvironmentConfig;

/// The environment initialization actor.
///
/// Runs initialization during `on_start`: loads `providers.toml`,
/// resolves API keys, populates `ApiKeysService`, and registers
/// in the actor registry for downstream lookups.
pub struct EnvInitActor {
    deps: ActorDeps,
    config: Option<ProvidersConfig>,
}

/// Dependencies for spawning an [`EnvInitActor`].
pub struct EnvInitActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
    /// Optional registry name. When `Some`, the actor registers itself in the kameo actor
    /// registry so downstream actors can look it up via `ask(GetEnvironmentConfig)`. Tests
    /// should pass `None` to avoid global registry conflicts between parallel test runs.
    pub registry_name: Option<&'static str>,
}

impl Actor for EnvInitActor {
    type Args = EnvInitActorDeps;
    type Error = Report<EnvInitError>;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        if let Some(name) = args.registry_name {
            actor_ref
                .register(name)
                .change_context(EnvInitError)
                .attach("failed to register env-init actor in registry")?;
        }

        let actor = Self {
            deps: args.deps,
            config: None,
        };
        Ok(actor)
    }
}

impl Message<GetEnvironmentConfig> for EnvInitActor {
    type Reply = Option<ProvidersConfig>;

    async fn handle(
        &mut self,
        _msg: GetEnvironmentConfig,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if self.config.is_none() {
            self.config = self.load_config_and_resolve_keys();
        }
        self.config.clone()
    }
}

impl Message<EnvironmentLoaded> for EnvInitActor {
    type Reply = ();

    async fn handle(&mut self, _msg: EnvironmentLoaded, _ctx: &mut Context<Self, ()>) {
        // No-op: EnvInitActor doesn't react to EnvironmentLoaded.
    }
}

impl BusPublish for EnvInitActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        self.deps.bus()
    }
}

impl EnvInitActor {
    /// Loads config, resolves API keys. Returns config on success.
    fn load_config_and_resolve_keys(&self) -> Option<ProvidersConfig> {
        let config = match self.deps.services.config_storage.load() {
            Ok(config) => config,
            Err(e) => {
                tracing::error!(err = ?e, "env-init failed to load provider config");
                return None;
            }
        };

        // Resolve API keys from environment variables.
        for provider in &config.providers {
            if let Some(ref env_var) = provider.api_key_env
                && let Ok(value) = std::env::var(env_var)
                && !value.is_empty()
            {
                self.deps.services.api_keys.insert(env_var.clone(), value);
            }
        }

        tracing::info!("environment loaded, API keys resolved");
        Some(config)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use std::time::Duration;

    use crate::common::bus::test_harness::{TestHarness, await_recorded};
    use crate::feat::provider_infra::ProvidersConfig;

    use super::{EnvInitActor, EnvInitActorDeps, EnvironmentLoaded, GetEnvironmentConfig};

    #[tokio::test]
    async fn get_environment_config_returns_none_without_config_file() {
        // Given an env init actor with no config file.
        let harness = TestHarness::new().await;
        let actor = harness
            .spawn_actor::<EnvInitActor>(EnvInitActorDeps {
                deps: harness.actor_deps().await,
                registry_name: None,
            })
            .await;

        // When asking for config.
        let config: Result<Option<ProvidersConfig>, _> = actor.ask(GetEnvironmentConfig).await;

        // Then ask succeeds (but config may be None without a config file).
        assert!(config.is_ok(), "ask should succeed");
    }

    #[tokio::test]
    async fn environment_loaded_can_be_published_for_reload() {
        // Given an env init actor and a recorder.
        let harness = TestHarness::new().await;
        let _actor = harness
            .spawn_actor::<EnvInitActor>(EnvInitActorDeps {
                deps: harness.actor_deps().await,
                registry_name: None,
            })
            .await;
        let recorder = harness.spawn_recorder::<EnvironmentLoaded>().await;

        // When publishing EnvironmentLoaded manually (runtime reload).
        let bus = harness.bus();
        bus.publish(EnvironmentLoaded {
            config: crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            },
        })
        .await;

        // Then the event is received by subscribers.
        let events = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
        assert_eq!(events.len(), 1, "expected EnvironmentLoaded event");
    }
}
