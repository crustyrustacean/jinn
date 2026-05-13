//! Environment initialization actor — reads env vars and populates API keys.
//!
//! Loads `providers.toml` to discover which environment variables hold API keys,
//! resolves those keys, populates the shared `ApiKeysService`, and emits
//! `EnvironmentLoaded` with the parsed config so downstream actors don't need
//! to reload the file.

use std::sync::Arc;

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::common::actor_host::{ActorSpawnResult, spawn_actor_impl};
use crate::feat::provider_infra::{ApiKeysService, ConfigStorageService, ProvidersConfig};
use crate::protocol::EventMsg;
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

/// Direct message type (unused — the env init actor only reacts to system messages).
pub enum EnvInitDirectMsg {}

/// The environment initialization actor.
///
/// Loads `providers.toml`, resolves API keys from environment variables,
/// populates `ApiKeysService`, and emits `EnvironmentLoaded`.
pub struct EnvInitActor {
    /// Config storage for loading `providers.toml`.
    config_storage: ConfigStorageService,
    /// API keys service to populate.
    api_keys: ApiKeysService,
}

impl Actor for EnvInitActor {
    type Message = EnvInitDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "Data injection is required at activation"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.set_description("Loads environment variables and API keys");

        let config_storage = ctx
            .take_data::<ConfigStorageService>()
            .expect("EnvInitActor requires ConfigStorageService injection");
        let api_keys = ctx
            .take_data::<ApiKeysService>()
            .expect("EnvInitActor requires ApiKeysService injection");

        Self {
            config_storage,
            api_keys,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Event(_)
            | ActorEnvelope::Command(_)
            | ActorEnvelope::Direct(_)
            | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}



/// Spawns the env init actor.
pub fn spawn_env_init_actor(
    config_storage: ConfigStorageService,
    api_keys: ApiKeysService,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<EnvInitDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<EnvInitDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("env-init", sink);
    ctx.set_description("Loads environment variables and API keys");
    ctx.set_data(config_storage);
    ctx.set_data(api_keys);
    let actor = EnvInitActor::activate(&mut ctx);
    let result = spawn_actor_impl("env-init", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::feat::provider_infra::{
        ApiKeys, ApiKeysService, ConfigStorageService, FilesystemConfigStorage,
    };
    use super::EnvInitActor;

    /// Creates a test actor with in-memory storage.
    fn create_actor() -> (
        EnvInitActor,
        ApiKeysService,
        Arc<RecordingSink>,
        ActorContext,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("env-init", sink.clone() as Arc<dyn MessageSink>);

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("providers.toml");
        let storage = FilesystemConfigStorage::new(path);

        let config_storage = ConfigStorageService::new(Arc::new(storage));
        let api_keys = ApiKeysService::new(ApiKeys::new());

        ctx.set_data(config_storage);
        ctx.set_data(api_keys.clone());
        let actor = EnvInitActor::activate(&mut ctx);
        (actor, api_keys, sink, ctx)
    }

    // Note: The ApplicationReady → on_application_ready flow has been removed.
    // Init logic will be replaced with self-scheduling in Phase 2.
}
