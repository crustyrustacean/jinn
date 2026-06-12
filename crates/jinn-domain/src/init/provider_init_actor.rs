//! Provider initialization actor - loads static config, merges cache, resolves `last_model`.
//!
//! Subscribes to [`EnvironmentLoaded`](super::EnvironmentLoaded) emitted by the
//! env init actor. On receipt: builds the `ProviderRegistry` from the config,
//! replaces the empty startup registry, loads the model cache from disk, merges
//! cache entries into the registry, loads app state, and if `last_model`
//! is set, sends a `ProviderSwitch` command to apply it.

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::provider::protocol::command::ProviderSwitch;
use crate::feat::provider::protocol::event::ModelCacheLoaded;
use crate::feat::provider_infra::{ModelCache, ProviderRegistry};
use crate::init::EnvironmentLoaded;
use kameo::prelude::{Actor, ActorRef, Context, Message};

/// The provider initialization actor.
///
/// On `EnvironmentLoaded`: builds the registry from config, replaces the empty
/// registry in `ProviderRegistryService`, loads the model cache, merges into
/// registry, loads app state, and sends `ProviderSwitch` if `last_model`
/// is set.
pub struct ProviderInitActor {
    /// Shared dependencies.
    deps: ActorDeps,
    /// Shared application state (to read active session ID).
    state: State,
}

/// Dependencies for [`ProviderInitActor`].
pub struct ProviderInitActorDeps {
    /// Shared dependencies.
    pub deps: ActorDeps,
    /// Shared application state.
    pub state: State,
}

impl Actor for ProviderInitActor {
    type Args = ProviderInitActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<EnvironmentLoaded>())
            .await;
        Ok(Self {
            deps: args.deps,
            state: args.state,
        })
    }
}

impl BusPublish for ProviderInitActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

impl Message<EnvironmentLoaded> for ProviderInitActor {
    type Reply = ();

    async fn handle(&mut self, msg: EnvironmentLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_environment_loaded(&msg.config).await;
    }
}

impl ProviderInitActor {
    /// Builds registry, merges cache, resolves `last_model`.
    async fn on_environment_loaded(&self, config: &crate::feat::provider_infra::ProvidersConfig) {
        // Build registry from config and replace the empty one.
        let registry = match ProviderRegistry::from_config(config.clone()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(err = ?e, "provider-init failed to build registry from config");
                return;
            }
        };
        self.deps.services.provider_registry.replace(registry);

        // Check if no API keys were resolved. If so, push a guidance message.
        if self.deps.services.api_keys.is_empty() {
            tracing::warn!("no API keys found, showing guidance message");
            let session_id = self.state.read().session.active_session_id().clone();
            self.publish(PushChatEntry {
                session_id,
                entry: crate::feat::session::no_api_keys_msg(),
            })
            .await;
        }

        // Load model cache from disk and merge into registry.
        let cache_path = self.deps.services.paths.cache_path();
        let cache = ModelCache::load(&cache_path).unwrap_or_else(|e| {
            tracing::warn!("provider-init failed to load model cache: {e:?}");
            None
        });
        if let Some(ref c) = cache {
            tracing::info!(providers = c.entries.len(), "loaded model cache");
            self.deps.services.provider_registry.merge_cache(c);
            self.publish(ModelCacheLoaded { cache: c.clone() }).await;
        }
        self.state.write().provider.model_cache = cache;

        let app_state = self.deps.services.app_state_storage.read();

        // If last_model is set, send ProviderSwitch to apply it.
        // Skip if the active session already has an explicit model (e.g., bench sessions
        // created with a CLI-specified model). Those sessions must keep their model.
        let active_session_model = {
            let state = self.state.read();
            state.active_session().profile().model.clone()
        };
        if active_session_model == crate::feat::provider_infra::NO_PROVIDER_ID
            && let Some(ref model) = app_state.last_model
        {
            let id = crate::feat::provider_infra::ProviderId::new(model.clone());
            let is_available = {
                let api_keys = self.deps.services.api_keys.read();
                self.deps
                    .services
                    .provider_registry
                    .is_available(&id, &api_keys)
            };
            if is_available {
                let session_id = self.state.read().session.active_session_id().clone();
                tracing::info!(last_model = %model, "provider-init resolving last_model");
                self.publish(ProviderSwitch {
                    session_id,
                    provider_id: model.clone(),
                })
                .await;
            } else {
                tracing::warn!(last_model = %model, "provider-init: last_model not available, skipping");
            }
        }
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

    use crate::AppState;
    use crate::common::bus::test_harness::TestHarness;
    use crate::common::state::State;
    use crate::feat::chat_input::protocol::command::PushChatEntry;
    use crate::feat::provider::protocol::command::ProviderSwitch;
    use crate::feat::provider::protocol::event::ModelCacheLoaded;
    use crate::feat::provider_infra::ProviderEntry;
    use crate::init::EnvironmentLoaded;

    use super::{ProviderInitActor, ProviderInitActorDeps};
    use crate::common::actor_deps::ActorDeps;
    use kameo::prelude::Spawn;

    fn sample_config() -> crate::feat::provider_infra::ProvidersConfig {
        crate::feat::provider_infra::ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "sample".to_owned(),
                backend: "sample".to_owned(),
                models: vec!["sample".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
                context_length: None,
            }],
            aliases: vec![],
            default_provider: None,
        }
    }

    fn ollama_config() -> crate::feat::provider_infra::ProvidersConfig {
        crate::feat::provider_infra::ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                models: vec!["llama3".to_owned()],
                base_url: None,
                api_key_env: None,
                requires_key: false,
                extra_body: None,
                context_length: None,
            }],
            aliases: vec![],
            default_provider: None,
        }
    }

    #[tokio::test]
    async fn sends_provider_switch_when_last_model_set() {
        // Given a provider init actor with preferences containing last_model.
        let harness = TestHarness::new().await;
        let services = harness.services().await;

        services
            .app_state_storage
            .save(
                &crate::feat::preferences_actor::app_state_file::AppStateFile {
                    last_model: Some("sample/sample".to_owned()),
                    ..Default::default()
                },
            )
            .expect("save app state");

        let state = State::new(AppState::default());
        let _actor = ProviderInitActor::spawn(ProviderInitActorDeps {
            deps: ActorDeps {
                services: services.clone(),
            },
            state: state.clone(),
        });

        let recorder = harness.spawn_recorder::<ProviderSwitch>().await;

        // When publishing EnvironmentLoaded.
        harness
            .publish(EnvironmentLoaded {
                config: sample_config(),
            })
            .await;

        let recorded = crate::common::bus::test_harness::await_recorded(
            &recorder,
            1,
            std::time::Duration::from_secs(2),
        )
        .await;

        // Then a ProviderSwitch command was sent.
        let found = recorded.iter().any(|c| c.provider_id == "sample/sample");
        assert!(found, "expected ProviderSwitch command for sample/sample");
    }

    #[tokio::test]
    async fn does_not_send_provider_switch_when_no_last_model() {
        // Given a provider init actor with no last_model in preferences.
        let harness = TestHarness::new().await;
        let services = harness.services().await;
        let state = State::new(AppState::default());
        let _actor = ProviderInitActor::spawn(ProviderInitActorDeps {
            deps: ActorDeps {
                services: services.clone(),
            },
            state,
        });

        let recorder = harness.spawn_recorder::<ProviderSwitch>().await;

        // When publishing EnvironmentLoaded.
        harness
            .publish(EnvironmentLoaded {
                config: sample_config(),
            })
            .await;

        let recorded = crate::common::bus::test_harness::await_recorded(
            &recorder,
            1,
            std::time::Duration::from_millis(500),
        )
        .await;

        // Then no ProviderSwitch command was sent.
        assert!(recorded.is_empty(), "expected no ProviderSwitch command");
    }

    #[tokio::test]
    async fn pushes_no_api_keys_msg_when_keys_empty() {
        // Given a provider init actor with no API keys and a provider that requires one.
        let harness = TestHarness::new().await;
        let services = harness.services().await;
        let state = State::new(AppState::default());
        let _actor = ProviderInitActor::spawn(ProviderInitActorDeps {
            deps: ActorDeps {
                services: services.clone(),
            },
            state,
        });

        let recorder = harness.spawn_recorder::<PushChatEntry>().await;

        let config = crate::feat::provider_infra::ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "openrouter".to_owned(),
                backend: "openrouter".to_owned(),
                models: vec!["gpt-4".to_owned()],
                base_url: None,
                api_key_env: Some("OPENROUTER_API_KEY".to_owned()),
                requires_key: true,
                extra_body: None,
                context_length: None,
            }],
            aliases: vec![],
            default_provider: None,
        };

        // When publishing EnvironmentLoaded with no API keys resolved.
        harness.publish(EnvironmentLoaded { config }).await;

        let recorded = crate::common::bus::test_harness::await_recorded(
            &recorder,
            1,
            std::time::Duration::from_secs(2),
        )
        .await;

        // Then a PushChatEntry command was emitted with the no-api-keys guidance.
        let has_no_api_keys = recorded
            .iter()
            .any(|cmd| cmd.entry.text().contains("No API keys found"));
        assert!(
            has_no_api_keys,
            "expected PushChatEntry with no-api-keys guidance"
        );
    }

    #[tokio::test]
    async fn emits_model_cache_loaded_when_cache_exists_on_disk() {
        // Given a provider init actor with a cache file on disk.
        let harness = TestHarness::new().await;
        let services = harness.services().await;

        let mut cache = crate::feat::provider_infra::ModelCache::new();
        cache.entries.insert(
            "ollama".to_owned(),
            vec![crate::feat::provider_infra::ModelInfo {
                id: "llama3".to_owned(),
                context_length: None,
            }],
        );
        cache.last_updated_at = Some(jiff::Timestamp::now());
        let cache_path = services.paths.cache_path();
        cache.save(&cache_path).expect("save cache");

        let state = State::new(AppState::default());
        let _actor = ProviderInitActor::spawn(ProviderInitActorDeps {
            deps: ActorDeps {
                services: services.clone(),
            },
            state,
        });

        let recorder = harness.spawn_recorder::<ModelCacheLoaded>().await;

        // When publishing EnvironmentLoaded.
        harness
            .publish(EnvironmentLoaded {
                config: ollama_config(),
            })
            .await;

        let recorded = crate::common::bus::test_harness::await_recorded(
            &recorder,
            1,
            std::time::Duration::from_secs(2),
        )
        .await;

        // Then a ModelCacheLoaded event was emitted.
        let found = recorded
            .iter()
            .any(|e| e.cache.entries.contains_key("ollama"));
        assert!(found, "expected ModelCacheLoaded event with ollama entries");
    }

    #[tokio::test]
    async fn does_not_send_provider_switch_when_session_has_explicit_model() {
        // Given a provider init actor with app state containing last_model
        // but the active session already has an explicitly set model.
        let harness = TestHarness::new().await;
        let services = harness.services().await;
        let state = State::new(AppState::default());

        // Set an explicit model on the active session (simulating bench actor).
        state
            .write()
            .active_session_mut()
            .set_model("bench-model".to_owned());

        // Set up app state with a last_model (should be ignored since session has explicit model).
        services
            .app_state_storage
            .save(
                &crate::feat::preferences_actor::app_state_file::AppStateFile {
                    last_model: Some("sample/sample".to_owned()),
                    ..Default::default()
                },
            )
            .expect("save app state");

        let _actor = ProviderInitActor::spawn(ProviderInitActorDeps {
            deps: ActorDeps {
                services: services.clone(),
            },
            state,
        });

        let recorder = harness.spawn_recorder::<ProviderSwitch>().await;

        // When publishing EnvironmentLoaded.
        harness
            .publish(EnvironmentLoaded {
                config: sample_config(),
            })
            .await;

        let recorded = crate::common::bus::test_harness::await_recorded(
            &recorder,
            1,
            std::time::Duration::from_millis(500),
        )
        .await;

        // Then no ProviderSwitch command was sent (session model was preserved).
        assert!(
            recorded.is_empty(),
            "expected no ProviderSwitch when session already has explicit model"
        );
    }
}
