//! Environment initialization actor - reads env vars and populates API keys.
//!
//! During startup (`on_start`), loads `providers.toml`, resolves API keys from
//! environment variables, populates the shared `ApiKeysService`, and publishes
//! `EnvironmentLoaded` with the parsed config so downstream actors
//! (provider_init) can use it without reloading the file.

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::bus::BusMessage;
use crate::common::actor::event_msg::EventMsg;
use crate::feat::provider_infra::ProvidersConfig;
use crate::protocol;
use kameo::prelude::{Actor, ActorRef};
use wherror::Error;

/// Error type for environment initialization failures.
#[derive(Debug, Error)]
#[error(debug)]
pub struct EnvInitError;

/// The environment has been loaded and API keys are available.
///
/// Emitted after the env init actor has populated `ApiKeysService`.
/// Carries the parsed `ProvidersConfig` so downstream actors (provider_init)
/// can use it without reloading the file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, protocol::EventMsg)]
#[event_msg("environment")]
pub struct EnvironmentLoaded {
    /// The parsed provider configuration from `providers.toml`.
    pub config: ProvidersConfig,
}

impl BusMessage for EnvironmentLoaded {}

/// The environment initialization actor.
///
/// Runs initialization during `on_start`: loads `providers.toml`,
/// resolves API keys, populates `ApiKeysService`, and publishes
/// `EnvironmentLoaded` on the bus.
pub struct EnvInitActor {
    deps: ActorDeps,
}

/// Dependencies for spawning an [`EnvInitActor`].
pub struct EnvInitActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
}

impl Actor for EnvInitActor {
    type Args = EnvInitActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, _actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let actor = Self { deps: args.deps };
        let config = actor.load_config_and_resolve_keys();
        match config {
            Some(config) => {
                actor.publish(EnvironmentLoaded { config }).await;
            }
            None => {
                tracing::error!("env-init failed to load provider config");
            }
        }
        Ok(actor)
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

    use super::{EnvInitActor, EnvInitActorDeps, EnvironmentLoaded};

    #[tokio::test]
    async fn initialize_publishes_environment_loaded() {
        // Given an env init actor.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<EnvironmentLoaded>().await;

        let _actor = harness
            .spawn_actor::<EnvInitActor>(EnvInitActorDeps {
                deps: harness.actor_deps().await,
            })
            .await;

        // Then an EnvironmentLoaded event was published.
        let events = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
        assert_eq!(events.len(), 1, "expected EnvironmentLoaded event");
    }
}
