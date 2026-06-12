//! Actor wiring — spawns all actors as kameo actors.
//!
//! This module encapsulates the one-time startup wiring: creating shared state,
//! spawning each actor via kameo's `Spawn::spawn()`, building the bus and bridge,
//! and waiting for the actor system to become ready. Called once from `App::dispatch`.
//!
//! # Spawn order
//!
//! 1. Infrastructure actors (system-ready, env-init).
//! 2. Init actors (provider-init, preferences, scan actors).
//! 3. Domain actors (session, tools, history workers, etc.).
//!
//! EnvInitActor is spawned first with `wait_for_startup()` so dependent actors
//! can look it up in the kameo registry and pull config via `ask()`.

use jinn_domain::ApiKeysService;
use jinn_domain::AppState;
use jinn_domain::ConfigStorageService;
use jinn_domain::LlmServiceFactoryService;
use jinn_domain::ProviderRegistryService;
use jinn_domain::Services;
use jinn_domain::SessionStoreService;
use jinn_domain::UserPreferencesStorageService;

use jinn_domain::actor_channel::ActorChannelService;
use jinn_domain::common::actor_deps::ActorDeps;
use jinn_domain::feat::context::strategy::token_estimator::TiktokenCounter;

use jinn_domain::feat::plugin_dispatch::{PluginDispatchActor, PluginDispatchActorDeps};
use jinn_domain::init::env_init_actor::{EnvInitActor, EnvInitActorDeps};
use jinn_domain::init::provider_init_actor::{ProviderInitActor, ProviderInitActorDeps};
use jinn_domain::init::system_ready_actor::{SystemReadyActor, SystemReadyActorDeps};

use jinn_domain::feat::preferences_actor::user_preferences::WebFetchBackend;
use jinn_domain::feat::web_fetch_actor::{WebFetchActor, WebFetchActorDeps};
use jinn_web_fetch::{HttpFetcher, MarkdownExtractor, OutputFormat};

use jinn_domain::{AppCore, AppMsg, State};

use kameo::actor::Spawn;

/// The fixed (required) inputs to actor-system construction.
///
/// These are needed for every launch; the only opt-in beyond them is
/// [`ActorSystemBuilder::with_bench_actor`].
#[derive(Clone)]
pub struct ActorSystemBuilderArgs {
    /// Tokio runtime handle actors are spawned onto.
    pub handle: tokio::runtime::Handle,
    /// LLM service factory.
    pub llm_service: LlmServiceFactoryService,
    /// Provider registry service.
    pub provider_registry: ProviderRegistryService,
    /// Resolved API keys.
    pub api_keys: ApiKeysService,
    /// Config storage service.
    pub config_storage: ConfigStorageService,
    /// Session store service. Caller-built (e.g. `SqliteSessionStore`).
    pub session_store: SessionStoreService,
    /// User preferences storage service.
    pub user_preferences_storage: UserPreferencesStorageService,
    /// App state storage service.
    pub app_state_storage: jinn_domain::feat::preferences_actor::AppStateStorageService,
    /// Application paths.
    pub paths: jinn_domain::AppPaths,
}

/// Opt-in bench inputs, set via [`ActorSystemBuilder::with_bench_actor`].
pub struct BenchInputs {
    /// Path to the bench CSV output.
    pub csv_path: std::path::PathBuf,
    /// The bench plan (model × task pairs).
    pub plan: jinn_bench::orchestrator::BenchPlan,
    /// Optional artifact directory.
    pub artifact_dir: Option<std::path::PathBuf>,
}

/// Builds the actor system: spawns all actors via kameo.
///
/// Construct with [`ActorSystemBuilder::new`], optionally add bench via
/// [`ActorSystemBuilder::with_bench_actor`], then call
/// [`ActorSystemBuilder::build`]. After spawning all actors, `build` blocks
/// the calling thread until the actor system signals readiness (3s timeout).
pub struct ActorSystemBuilder {
    args: ActorSystemBuilderArgs,
    bench: Option<BenchInputs>,
}

impl ActorSystemBuilder {
    /// Create a builder with the given fixed inputs. No bench actor.
    #[must_use]
    pub fn new(args: ActorSystemBuilderArgs) -> Self {
        Self { args, bench: None }
    }

    /// Opt-in: register the bench actor with the given plan.
    #[must_use]
    pub fn with_bench_actor(
        mut self,
        csv_path: std::path::PathBuf,
        plan: jinn_bench::orchestrator::BenchPlan,
        artifact_dir: Option<std::path::PathBuf>,
    ) -> Self {
        self.bench = Some(BenchInputs {
            csv_path,
            plan,
            artifact_dir,
        });
        self
    }

    /// Spawn all actors via kameo, build the bus and bridge, and wait for readiness.
    pub async fn build(self) -> (AppCore, Services, jinn_plugin::SyncPlugins) {
        let ActorSystemBuilderArgs {
            handle,
            llm_service,
            provider_registry,
            api_keys,
            config_storage,
            session_store,
            user_preferences_storage,
            app_state_storage,
            paths,
        } = self.args;
        let bench = self.bench;

        // Create channel first — actors need the sender, but AppCore needs services
        // which needs the bus. Break the cycle by creating the channel independently.
        let (sender, receiver) = kanal::unbounded::<AppMsg>();
        let _async_receiver = receiver.to_async();

        // `DomainNodeContext` is needed by the plugin request handler (for `llm_oneshot`)
        // but it can't be constructed until `services` is assembled.
        // Bridge with a `OnceLock` filled in once `services` exists.
        let domain_ctx_cell: std::sync::Arc<
            std::sync::OnceLock<jinn_domain::feat::plugin_dispatch::DomainNodeContext>,
        > = std::sync::Arc::new(std::sync::OnceLock::new());

        // Create shared State FIRST — injected into multiple actors.
        let state = State::new(AppState::default());

        // Set preferences
        {
            let mut guard = state.write();
            guard.frontend.preferences = user_preferences_storage.read();
        }

        // Set app state (last_model, theme_name, persona_name, sidebar_width)
        {
            let app_state = app_state_storage.read();
            let mut guard = state.write();
            guard.frontend.app_state.last_model = app_state.last_model.clone();
            guard.frontend.app_state.theme_name = app_state.theme_name.clone();
            guard.frontend.app_state.persona_name = app_state.persona_name.clone();
            guard.frontend.app_state.sidebar_width = app_state.sidebar_width;
        }

        // Set default CWD for sessions (inherited from shell).
        {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
            let mut guard = state.write();
            guard.session.set_default_cwd(cwd.clone());
            guard.active_session_mut().set_cwd(cwd);
        }

        // ── Plugin system ────────────────────────────────────────────────────
        // Constructed early — handles go into Services and TuiApp.

        //FIXME: disabled during actor migration — plugin command dispatch needs redesign for typed bus messages
        let plugin_command_dispatcher: jinn_plugin::CommandDispatcher = std::sync::Arc::new({
            let _sender = sender.clone();
            move |_cmd: jinn_plugin::PluginCommand| {
                // crate::plugin_wiring::handle_plugin_command(cmd, &sink);
            }
        });
        let handler_cell = domain_ctx_cell.clone();
        let plugin_request_handler: jinn_plugin::RequestHandler = std::sync::Arc::new({
            move |name: &str, data: &serde_json::Value| {
                let cell = handler_cell.clone();
                let name = name.to_string();
                let data = data.clone();
                std::boxed::Box::pin(async move {
                    match cell.get() {
                        Some(ctx) => {
                            crate::plugin_wiring::handle_plugin_request(&name, &data, ctx).await
                        }
                        None => {
                            tracing::warn!(
                                name,
                                "plugin request before domain_ctx ready; returning null"
                            );
                            serde_json::Value::Null
                        }
                    }
                })
            }
        });

        let (sync_plugins, async_plugins, plugin_sync_handle) = jinn_plugin::PluginSystem::build(
            &paths.plugins_dir(),
            &paths.system_plugins_dir(),
            handle.clone(),
            plugin_command_dispatcher,
            plugin_request_handler,
        );

        // Store discovered plugin metadata in state for the sidebar.
        {
            let plugins =
                jinn_plugin::discover_plugins(&paths.plugins_dir(), &paths.system_plugins_dir());
            tracing::info!(count = plugins.len(), "discovered plugins");
            let plugins: Vec<jinn_domain::common::app_state::DiscoveredPlugin> = plugins
                .into_iter()
                .map(|p| jinn_domain::common::app_state::DiscoveredPlugin {
                    name: p.name,
                    description: p.description,
                })
                .collect();
            state.write().discovered_plugins = plugins;
        }

        let domain_ctx_cell: std::sync::Arc<
            std::sync::OnceLock<jinn_domain::feat::plugin_dispatch::DomainNodeContext>,
        > = std::sync::Arc::new(std::sync::OnceLock::new());
        let _handler_cell = domain_ctx_cell.clone();

        // Create the kameo message bus and closure bridge first.
        let bus = {
            let bus_actor = kameo_actors::message_bus::MessageBus::new(
                kameo_actors::DeliveryStrategy::BestEffort,
            );
            let bus_ref = kameo_actors::message_bus::MessageBus::spawn(bus_actor);
            jinn_domain::common::services::bus_service::BusService::new(bus_ref)
        };
        let bridge = jinn_domain::common::bridge::Bridge::new(bus.actor_ref().clone());

        let services = Services {
            paths: paths.clone(),
            handle: handle.clone(),
            actor_channel: ActorChannelService::new(sender.clone()),
            llm_service: llm_service.clone(),
            provider_registry: provider_registry.clone(),
            api_keys: api_keys.clone(),
            config_storage: config_storage.clone(),
            session_store: session_store.clone(),
            user_preferences_storage: user_preferences_storage.clone(),
            app_state_storage: app_state_storage.clone(),
            plugins: jinn_domain::feat::plugin_dispatch::PluginFireService::new(
                std::sync::Arc::new(async_plugins.clone())
                    as std::sync::Arc<dyn jinn_domain::feat::plugin_dispatch::PluginFire>,
            ),
            plugin_sync: jinn_domain::feat::plugin_dispatch::PluginSyncCallService::new(
                std::sync::Arc::new(plugin_sync_handle)
                    as std::sync::Arc<dyn jinn_domain::feat::plugin_dispatch::PluginSyncCall>,
            ),
            session_plugin_registry:
                jinn_domain::feat::plugin_system::SessionPluginRegistryService::new(
                    std::sync::Arc::new(async_plugins.clone())
                        as std::sync::Arc<
                            dyn jinn_domain::feat::plugin_system::SessionPluginRegistry,
                        >,
                ),
            tempdir: None,
            bus,
            bridge,
        };

        // Now that `services` + `state` exist, build the shared `DomainNodeContext`
        // and publish it into the `OnceLock` so the plugin request handler can see it.
        let shared_domain_ctx =
            std::sync::Arc::new(jinn_domain::feat::plugin_dispatch::DomainNodeContext::new(
                services.clone(),
                state.clone(),
            ));
        let _ = domain_ctx_cell.set((*shared_domain_ctx).clone());

        let actor_deps = ActorDeps {
            services: services.clone(),
        };

        // ── Infrastructure actors ──────────────────────────────────────────

        // System-ready actor: signals main thread when all actors started.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let _system_ready = SystemReadyActor::spawn(SystemReadyActorDeps {
            deps: actor_deps.clone(),
            ready_tx,
        });

        // ── Init actors ────────────────────────────────────────────────────

        // Env init: registers in actor registry, defers config loading to GetEnvironmentConfig ask.
        let env_init = EnvInitActor::spawn(EnvInitActorDeps {
            deps: actor_deps.clone(),
            registry_name: Some("env-init"),
        });
        env_init.wait_for_startup().await;
        // Provider init: on EnvironmentLoaded, builds registry, merges cache, resolves last_model.
        let _provider_init = ProviderInitActor::spawn(ProviderInitActorDeps {
            deps: actor_deps.clone(),
            state: state.clone(),
        });

        // Preferences: loads and persists user preferences.
        let _preferences =
            jinn_domain::feat::preferences_actor::preferences_actor::PreferencesActor::spawn(
                jinn_domain::feat::preferences_actor::preferences_actor::PreferencesActorDeps {
                    deps: actor_deps.clone(),
                },
            );

        // Preferences state sync: updates AppState from PreferencesUpdated events.
        let _preferences_sync =
            jinn_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActor::spawn(
                jinn_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            );

        // App state actor: persists state changes to state.toml.
        let _app_state =
            jinn_domain::feat::preferences_actor::app_state_actor::AppStateActor::spawn(
                jinn_domain::feat::preferences_actor::app_state_actor::AppStateActorDeps {
                    deps: actor_deps.clone(),
                },
            );

        // App state sync: updates AppState from AppStateUpdated events.
        let _app_state_sync =
            jinn_domain::feat::preferences_actor::app_state_sync_actor::AppStateSyncActor::spawn(
                jinn_domain::feat::preferences_actor::app_state_sync_actor::AppStateSyncActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            );

        // ── Domain actors ──────────────────────────────────────────────────

        // LLM streaming actor.
        let _llm = jinn_domain::feat::llm_actor::LlmActor::spawn(
            jinn_domain::feat::llm_actor::LlmActorDeps {
                factory: llm_service.clone(),
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        );

        // Model discovery actor.
        let _discover = jinn_domain::feat::provider::discover_actor::DiscoverActor::spawn(
            jinn_domain::feat::provider::discover_actor::DiscoverActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        );

        // Session persistence actor — must spawn before ToolOrchestratorActor so
        // ToolsRegistered subscription is ready when tools register builtins in on_start.
        let token_counter = TiktokenCounter::o200k_base();
        let _session = jinn_domain::feat::session::session_actor::SessionPersistenceActor::spawn(
            jinn_domain::feat::session::session_actor::SessionPersistenceActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
                counter: token_counter,
                builtin_registry: {
                    let mut registry =
                        jinn_domain::feat::session_lifecycle::builtin::BuiltinRegistry::new();
                    jinn_bench::bench_tasks::register_bench_tasks(
                        &mut registry,
                        bench.as_ref().and_then(|b| b.artifact_dir.as_deref()),
                    );
                    registry
                },
                shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
            },
        );
        _session.wait_for_startup().await;

        // Tool orchestrator actor.
        let _tools = jinn_domain::feat::tools_actor::ToolOrchestratorActor::spawn(
            jinn_domain::feat::tools_actor::ToolOrchestratorActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
                services: services.clone(),
                builtin_filter: None,
                shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
            },
        );
        _tools.wait_for_startup().await;

        // Web fetch actor.
        let web_fetch_backend = user_preferences_storage.read().web_fetch.backend;
        tracing::info!(backend = ?web_fetch_backend, "constructing web fetcher");
        let extractors = {
            let markdown: std::sync::Arc<dyn jinn_web_fetch::Extractor> =
                std::sync::Arc::new(MarkdownExtractor);
            std::collections::HashMap::from([
                (OutputFormat::Text, markdown.clone()),
                (OutputFormat::Markdown, markdown),
            ])
        };
        let web_fetcher: std::sync::Arc<dyn jinn_web_fetch::WebFetcher> = match web_fetch_backend {
            WebFetchBackend::Http => {
                tracing::debug!("web-fetch: using HttpFetcher backend");
                std::sync::Arc::new(HttpFetcher::new(extractors.clone()))
            }
            WebFetchBackend::HeadlessChrome => {
                tracing::debug!("web-fetch: using HeadlessChromeFetcher backend");
                std::sync::Arc::new(jinn_web_fetch::HeadlessChromeFetcher::new(
                    extractors.clone(),
                ))
            }
        };
        let _web_fetch = WebFetchActor::spawn(WebFetchActorDeps {
            deps: actor_deps.clone(),
            web_fetcher,
        });

        // Prompt scan actor.
        let _prompt_scan = jinn_domain::feat::context::prompt_scan_actor::PromptScanActor::spawn(
            jinn_domain::feat::context::prompt_scan_actor::PromptScanActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        );

        // Context-files scan actor.
        let _context_files =
            jinn_domain::feat::context::context_files_scan_actor::ContextFilesScanActor::spawn(
                jinn_domain::feat::context::context_files_scan_actor::ContextFilesScanActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            );

        // Skills scan actor.
        let _skills_scan = jinn_domain::feat::skills::skills_scan_actor::SkillsScanActor::spawn(
            jinn_domain::feat::skills::skills_scan_actor::SkillsScanActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        );

        // Discovery coordinator.
        let _discovery_coordinator =
            jinn_domain::feat::discovery_coordinator::DiscoveryCoordinatorActor::spawn(
                jinn_domain::feat::discovery_coordinator::DiscoveryCoordinatorActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            );

        // Discovery notifier.
        let _discovery_notifier =
            jinn_domain::feat::discovery_notifier::DiscoveryNotifierActor::spawn(
                jinn_domain::feat::discovery_notifier::DiscoveryNotifierActorDeps {
                    deps: actor_deps.clone(),
                },
            );

        // Persona scan actor.
        let _persona_scan = jinn_domain::feat::persona::persona_scan_actor::PersonaScanActor::spawn(
            jinn_domain::feat::persona::persona_scan_actor::PersonaScanActorDeps {
                deps: actor_deps.clone(),
            },
        );

        // Provider actor.
        let _provider = jinn_domain::feat::provider::provider_actor::ProviderActor::spawn(
            jinn_domain::feat::provider::provider_actor::ProviderActorDeps {
                state: state.clone(),
                deps: actor_deps.clone(),
            },
        );

        let _token_count = jinn_domain::feat::token_count_actor::TokenCountActor::spawn(
            jinn_domain::feat::token_count_actor::TokenCountActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        );

        // Queue actor.
        let _queue = jinn_domain::feat::queue_actor::QueueActor::spawn(
            jinn_domain::feat::queue_actor::QueueActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
                counter: token_counter,
            },
        );

        // Context size actor.
        let _context_size = jinn_domain::feat::context::context_size_actor::ContextSizeActor::spawn(
            jinn_domain::feat::context::context_size_actor::ContextSizeActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
                counter: token_counter,
            },
        );

        // Echo actor.
        let _echo = jinn_domain::feat::echo_actor::EchoActor::spawn(
            jinn_domain::feat::echo_actor::EchoActorDeps {
                deps: actor_deps.clone(),
            },
        );

        // ── History mutation workers ───────────────────��──────────────────────
        //
        // To add a new history mutation worker:
        //
        //   1. Implement `HistoryWorker` for your heuristic type
        //      (see `crates/jinn-domain/src/feat/history_worker/worker_trait.rs`).
        //   2. Add a spawn call here following the pattern below.

        // History snapshot actor.
        {
            use jinn_domain::feat::history_worker::snapshot_actor::{
                HistorySnapshotActor, HistorySnapshotActorDeps,
            };

            let _snapshot = HistorySnapshotActor::spawn(HistorySnapshotActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
            });
        }

        // Compaction worker.
        {
            use jinn_domain::feat::compaction_worker::CompactionWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let _compaction =
                HistoryWorkerActor::<CompactionWorker>::spawn(HistoryWorkerActorDeps {
                    deps: actor_deps.clone(),
                    worker: CompactionWorker::new(services.clone(), handle.clone(), state.clone()),
                });
        }

        // Compaction trigger actor.
        {
            use jinn_domain::feat::compaction_worker::{
                CompactionTriggerActor, CompactionTriggerActorDeps, CompactionWorker,
            };

            let _trigger = CompactionTriggerActor::spawn(CompactionTriggerActorDeps {
                deps: actor_deps.clone(),
                worker: CompactionWorker::new(services.clone(), handle.clone(), state.clone()),
            });
        }

        // Auto-prune worker: read→edit context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::ReadEditAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.read_edit.clone();

            if config.enabled {
                let _worker =
                    HistoryWorkerActor::<ReadEditAutoPruneWorker>::spawn(HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: ReadEditAutoPruneWorker { config },
                    });
            }
        }

        // Auto-prune worker: edit→read context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::EditReadAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.edit_read.clone();

            if config.enabled {
                let _worker =
                    HistoryWorkerActor::<EditReadAutoPruneWorker>::spawn(HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: EditReadAutoPruneWorker { config },
                    });
            }
        }

        // Auto-prune worker: regex-based tool call pruning.
        {
            use jinn_domain::feat::auto_prune_worker::RegexAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };
            let regex_config = user_preferences_storage.read().auto_prune.regex.clone();

            if regex_config.enabled && !regex_config.rules.is_empty() {
                match RegexAutoPruneWorker::from_config(&regex_config) {
                    Ok(worker) => {
                        let _worker = HistoryWorkerActor::<RegexAutoPruneWorker>::spawn(
                            HistoryWorkerActorDeps {
                                deps: actor_deps.clone(),
                                worker,
                            },
                        );
                    }
                    Err(e) => {
                        tracing::warn!(err=?e, "invalid regex in auto_prune config, skipping");
                    }
                }
            } else {
                tracing::debug!(
                    enabled = regex_config.enabled,
                    rules = regex_config.rules.len(),
                    "regex auto-prune skipped",
                );
            }
        }

        // Auto-prune worker: todo tool call pruning.
        {
            use jinn_domain::feat::auto_prune_worker::TodoAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.todo.clone();

            if config.enabled {
                let _worker =
                    HistoryWorkerActor::<TodoAutoPruneWorker>::spawn(HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: TodoAutoPruneWorker { config },
                    });
            }
        }

        // Auto-prune worker: broken-edit context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::BrokenEditAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage
                .read()
                .auto_prune
                .broken_edit
                .clone();

            if config.enabled {
                let _worker = HistoryWorkerActor::<BrokenEditAutoPruneWorker>::spawn(
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: BrokenEditAutoPruneWorker { config },
                    },
                );
            }
        }

        // Auto-prune worker: double-edit context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::DoubleEditAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage
                .read()
                .auto_prune
                .double_edit
                .clone();

            if config.enabled {
                let _worker = HistoryWorkerActor::<DoubleEditAutoPruneWorker>::spawn(
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: DoubleEditAutoPruneWorker { config },
                    },
                );
            }
        }

        // Auto-prune worker: consecutive-reads per-file pruning.
        {
            use jinn_domain::feat::auto_prune_worker::ConsecutiveReadsAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage
                .read()
                .auto_prune
                .consecutive_reads
                .clone();

            if config.enabled {
                let _worker = HistoryWorkerActor::<ConsecutiveReadsAutoPruneWorker>::spawn(
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: ConsecutiveReadsAutoPruneWorker { config },
                    },
                );
            }
        }

        // Shared entry-token cache for history workers.
        let entry_token_cache =
            jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCache::new();

        // HistoryWorkerChatEntryTokenCache eviction actor.
        {
            use jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCacheEvictionActor;
            use jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCacheEvictionActorDeps;

            let _eviction = HistoryWorkerChatEntryTokenCacheEvictionActor::spawn(
                HistoryWorkerChatEntryTokenCacheEvictionActorDeps {
                    deps: actor_deps.clone(),
                    cache: entry_token_cache.clone(),
                },
            );
        }

        // Auto-prune worker: tool-age-window context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::ToolAgeWindowAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage
                .read()
                .auto_prune
                .tool_age_window
                .clone();

            if config.enabled {
                let _worker = HistoryWorkerActor::<ToolAgeWindowAutoPruneWorker>::spawn(
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: ToolAgeWindowAutoPruneWorker { config },
                    },
                );
            }
        }

        // Auto-prune worker: trivial-assistant context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::TrivialAssistantAutoPruneWorker;
            use jinn_domain::feat::context::strategy::token_estimator::TiktokenCounter;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage
                .read()
                .auto_prune
                .trivial_assistant
                .clone();

            if config.enabled {
                let _worker = HistoryWorkerActor::<TrivialAssistantAutoPruneWorker>::spawn(
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: TrivialAssistantAutoPruneWorker {
                            config,
                            token_cache: entry_token_cache.clone(),
                            counter: TiktokenCounter::o200k_base(),
                        },
                    },
                );
            }
        }

        // Auto-prune worker: anchor shield.
        {
            use jinn_domain::feat::auto_prune_worker::AnchorShieldAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let shield_config = {
                let prefs = user_preferences_storage.read();
                prefs.auto_prune.anchor_shield.clone()
            };

            if shield_config.enabled {
                let _worker = HistoryWorkerActor::<AnchorShieldAutoPruneWorker>::spawn(
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: AnchorShieldAutoPruneWorker {
                            config: shield_config,
                        },
                    },
                );
            }
        }

        // Auto-prune worker: anchored-assistant context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::AnchoredAssistantAutoPruneWorker;
            use jinn_domain::feat::context::strategy::token_estimator::TiktokenCounter;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let (config, shield_radius, trivial_max_tokens) = {
                let prefs = user_preferences_storage.read();
                let cfg = prefs.auto_prune.anchored_assistant.clone();
                let radius = prefs.auto_prune.anchor_shield.radius;
                let max_tokens = prefs.auto_prune.trivial_assistant.max_tokens as u32;
                (cfg, radius, max_tokens)
            };

            if config.enabled {
                let _worker = HistoryWorkerActor::<AnchoredAssistantAutoPruneWorker>::spawn(
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: AnchoredAssistantAutoPruneWorker {
                            config,
                            radius: shield_radius,
                            min_candidate_tokens: trivial_max_tokens + 1,
                            token_cache: entry_token_cache.clone(),
                            counter: TiktokenCounter::o200k_base(),
                        },
                    },
                );
            }
        }

        // Sidebar state actor.
        let _sidebar =
            jinn_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActor::spawn(
                jinn_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            );

        // Plugin dispatch actor.
        let _plugin_dispatch = PluginDispatchActor::spawn(PluginDispatchActorDeps {
            deps: actor_deps.clone(),
            services: services.clone(),
            state: state.clone(),
            startup_session_id: state.read().session.active_session_id().to_string(),
            domain_ctx: shared_domain_ctx.clone(),
        });

        // ── Bench actor (conditional) ─────────────────────────────────────────
        if let Some(b) = bench {
            let _bench = jinn_bench::bench_actor::BenchActor::spawn(
                jinn_bench::bench_actor::BenchActorKameoDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                    csv_path: Some(b.csv_path.clone()),
                    plan: Some(b.plan),
                },
            );
        }

        // Signal system readiness and trigger init chain.
        {
            let bus_ref = services.bus.actor_ref();
            let env_init = env_init.clone();

            // Signal all actors spawned.
            let _ = bus_ref
                .tell(kameo_actors::message_bus::Publish(
                    jinn_domain::common::actor::protocol::event::AllActorsSpawned,
                ))
                .await;

            // Ask EnvInitActor for config and publish EnvironmentLoaded to trigger init chain.
            use jinn_domain::init::env_init_actor::GetEnvironmentConfig;
            match env_init.ask(GetEnvironmentConfig).await {
                Ok(Some(config)) => {
                    let _ = bus_ref
                        .tell(kameo_actors::message_bus::Publish(
                            jinn_domain::init::env_init_actor::EnvironmentLoaded { config },
                        ))
                        .await;
                }
                Ok(None) => {
                    tracing::warn!("no provider config found — skipping EnvironmentLoaded");
                }
                Err(e) => {
                    tracing::error!(err = ?e, "failed to get environment config from EnvInitActor");
                }
            }
        }

        // Wait for SystemReadyActor to confirm readiness.
        let _ = ready_rx.await;

        // Build AppCore with shared state and sender.
        let core = AppCore {
            state: state.clone(),
            sender: sender.clone(),
            bridge: services.bridge.clone(),
        };

        (core, services, sync_plugins)
    }
}
