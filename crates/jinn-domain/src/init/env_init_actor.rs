//! Environment initialization actor - reads env vars and populates API keys.
//!
//! Self-schedules an [`EnvInitDirectMsg::Initialize`] message during activation.
//! On receipt: loads `providers.toml`, resolves API keys from environment
//! variables, populates the shared `ApiKeysService`, and emits
//! `EnvironmentLoaded` with the parsed config so downstream actors
//! (provider_init) can use it without reloading the file.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope};
use crate::common::services::Services;
use crate::feat::provider_infra::ProvidersConfig;
use crate::protocol::{Event, EventMsg};
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, EventMsg)]
#[event_msg("init")]
pub struct EnvironmentLoaded {
    /// The parsed provider configuration from `providers.toml`.
    pub config: ProvidersConfig,
}

/// Direct messages for the environment initialization actor.
pub enum EnvInitDirectMsg {
    /// Trigger initialization: load config and resolve API keys.
    Initialize,
}

/// The environment initialization actor.
///
/// Self-schedules `Initialize` during activation. On receipt, loads
/// `providers.toml`, resolves API keys, populates `ApiKeysService`,
/// and emits `EnvironmentLoaded`.
pub struct EnvInitActor {
    /// Runtime services.
    services: Services,
}

/// Dependencies for [`EnvInitActor`].
pub struct EnvInitActorDeps {
    /// Runtime services.
    pub services: Services,
}

impl Actor for EnvInitActor {
    type Message = EnvInitDirectMsg;
    type Deps = EnvInitActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Loads environment variables and API keys");

        // Self-schedule initialization - the message buffers until the run loop starts.
        #[expect(
            clippy::expect_used,
            reason = "self-ref is injected by spawn before activate"
        )]
        let self_ref = ctx
            .take_actor_ref::<EnvInitDirectMsg>()
            .expect("EnvInitActor requires self-ref injection");
        let _ = self_ref.send(EnvInitDirectMsg::Initialize);

        Self {
            services: deps.services,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Direct(EnvInitDirectMsg::Initialize) => {
                self.on_initialize(ctx);
            }
            ActorEnvelope::Event(_) | ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }

    async fn shutdown(self) {}
}

impl EnvInitActor {
    /// Loads config, resolves API keys, emits `EnvironmentLoaded`.
    fn on_initialize(&self, ctx: &ActorContext) {
        let config = match self.services.config_storage.load() {
            Ok(config) => config,
            Err(e) => {
                tracing::error!(err = ?e, "env-init failed to load provider config");
                return;
            }
        };

        // Resolve API keys from environment variables.
        for provider in &config.providers {
            if let Some(ref env_var) = provider.api_key_env
                && let Ok(value) = std::env::var(env_var)
                && !value.is_empty()
            {
                self.services.api_keys.insert(env_var.clone(), value);
            }
        }

        tracing::info!("environment loaded, API keys resolved");

        if let Err(e) = ctx.send_event(Event::EnvironmentLoaded(EnvironmentLoaded { config })) {
            tracing::error!(err = ?e, "env-init failed to emit EnvironmentLoaded");
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use std::sync::Arc;

    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, ActorRef, MessageSink, RecordingSink,
    };
    use crate::feat::provider_infra::{ApiKeysService, FilesystemConfigStorage};

    use super::{EnvInitActor, EnvInitActorDeps};

    /// Creates a test actor with in-memory storage.
    fn create_actor() -> (
        EnvInitActor,
        ApiKeysService,
        Arc<RecordingSink>,
        ActorContext,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("env-init", sink.clone() as Arc<dyn MessageSink>);
        ctx.set_actor_ref(ActorRef::new(
            kanal::unbounded::<ActorEnvelope<super::EnvInitDirectMsg>>().0,
        ));

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("providers.toml");
        let _storage = FilesystemConfigStorage::new(path);

        let services = crate::common::services::test_services::TestServices::builder().build();
        let api_keys = services.api_keys.clone();
        let deps = EnvInitActorDeps { services };
        let actor = EnvInitActor::activate(deps, &mut ctx);
        (actor, api_keys, sink, ctx)
    }
    #[tokio::test]
    async fn initialize_emits_environment_loaded() {
        // Given an env init actor.
        let (mut actor, _api_keys, sink, ctx) = create_actor();

        // When processing Initialize.
        actor
            .handle(
                ActorEnvelope::Direct(super::EnvInitDirectMsg::Initialize),
                &ctx,
            )
            .await;

        // Then an EnvironmentLoaded event was emitted.
        let events = sink.events();
        let found = events
            .iter()
            .any(|e| matches!(e, crate::protocol::Event::EnvironmentLoaded(..)));
        assert!(found, "expected EnvironmentLoaded event");
    }
}
