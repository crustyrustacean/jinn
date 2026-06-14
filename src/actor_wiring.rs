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

use jinn_domain::common::actor_deps::ActorDeps;
use jinn_domain::feat::context::strategy::token_estimator::TiktokenCounter;

use jinn_domain::feat::plugin_dispatch::{PluginDispatchActor, PluginDispatchActorDeps};
use jinn_domain::init::env_init_actor::{EnvInitActor, EnvInitActorDeps};
use jinn_domain::init::provider_init_actor::{ProviderInitActor, ProviderInitActorDeps};
use jinn_domain::init::system_ready_actor::{SystemReadyActor, SystemReadyActorDeps};

use jinn_domain::feat::preferences_actor::user_preferences::WebFetchBackend;
use jinn_domain::feat::web_fetch_actor::{WebFetchActor, WebFetchActorDeps};
use jinn_web_fetch::{HttpFetcher, MarkdownExtractor, OutputFormat};

use jinn_domain::{AppCore, State};

use kameo::actor::Spawn;

/// The fixed (required) inputs to actor-system construction.
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

/// Builds the actor system: spawns all actors via kameo.
///
/// Construct with [`ActorSystemBuilder::new`], then call
/// [`ActorSystemBuilder::build`]. After spawning all actors, `build` blocks
/// the calling thread until the actor system signals readiness (3s timeout).
pub struct ActorSystemBuilder {
    args: ActorSystemBuilderArgs,
}

impl ActorSystemBuilder {
    #[must_use]
    pub fn new(args: ActorSystemBuilderArgs) -> Self {
        Self { args }
    }
    /// Spawn all actors via kameo, build the bus and bridge, and wait for readiness.
    pub async fn build(
        self,
    ) -> (
        AppCore,
        Services,
        jinn_domain::feat::plugin_system::SyncPlugins,
    ) {
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

        // Create the kameo message bus and closure bridge first — the plugin
        // command dispatcher needs a Bridge to route commands onto the bus.
        let bus = {
            let bus_actor = kameo_actors::message_bus::MessageBus::new(
                kameo_actors::DeliveryStrategy::BestEffort,
            );
            let bus_ref = kameo_actors::message_bus::MessageBus::spawn(bus_actor);
            jinn_domain::common::services::bus_service::BusService::new(bus_ref)
        };
        let bridge = jinn_domain::common::bridge::Bridge::new(bus.actor_ref().clone());

        // ── Plugin system ─��──────────────────────────────────────────────────
        // Constructed early — handles go into Services and TuiApp.

        let plugin_command_dispatcher: jinn_domain::feat::plugin_system::CommandDispatcher =
            crate::plugin_wiring::build_command_dispatcher(bridge.clone());
        let handler_cell = domain_ctx_cell.clone();
        let plugin_request_handler: jinn_domain::feat::plugin_system::RequestHandler =
            std::sync::Arc::new({
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

        let plugin_build = jinn_domain::feat::plugin_system::PluginSystem::build(
            &paths.plugins_dir(),
            &paths.system_plugins_dir(),
            handle.clone(),
            plugin_command_dispatcher,
            plugin_request_handler,
        );

        let sync_plugins = plugin_build.sync;
        let async_plugins = plugin_build.async_handle;
        let plugin_sync_handle = plugin_build.sync_handle;
        let global_tool_metadata = plugin_build.global_tool_metadata;

        // Store discovered plugin metadata in state for the sidebar.
        {
            let plugins = jinn_domain::feat::plugin_system::discover_plugins(
                &paths.plugins_dir(),
                &paths.system_plugins_dir(),
            );
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

        // Root supervision tree: every spawned actor becomes a supervised
        // child so that stopping the root cascades a graceful shutdown.
        let root = jinn_domain::common::root_supervisor::RootSupervisor::spawn_root().await;

        let services = Services {
            paths: paths.clone(),
            handle: handle.clone(),
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
            root_supervisor: root.clone(),
        };

        // Global-scoped plugin tools will be registered after the tools actor spawns.
        // Now that `services` + `state` exist, build the shared `DomainNodeContext`
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
        let (ready_tx, ready_rx) = kanal::unbounded::<()>();
        let _system_ready = SystemReadyActor::supervise(
            &root,
            SystemReadyActorDeps {
                deps: actor_deps.clone(),
                ready_tx,
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;

        // ── Init actors ────────────────────────────────────────────────────

        // Env init: registers in actor registry, defers config loading to GetEnvironmentConfig ask.
        let env_init = EnvInitActor::supervise(
            &root,
            EnvInitActorDeps {
                deps: actor_deps.clone(),
                registry_name: Some("env-init"),
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;
        env_init.wait_for_startup().await;
        // Provider init: on EnvironmentLoaded, builds registry, merges cache, resolves last_model.
        let _provider_init = ProviderInitActor::supervise(
            &root,
            ProviderInitActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;

        // Preferences: loads and persists user preferences.
        let _preferences =
            jinn_domain::feat::preferences_actor::preferences_actor::PreferencesActor::supervise(
                &root,
                jinn_domain::feat::preferences_actor::preferences_actor::PreferencesActorDeps {
                    deps: actor_deps.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

        // Preferences state sync: updates AppState from PreferencesUpdated events.
        let _preferences_sync =
            jinn_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActor::supervise(&root,
                jinn_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            ).restart_policy(kameo::supervision::RestartPolicy::Never).spawn().await;

        // App state actor: persists state changes to state.toml.
        let _app_state =
            jinn_domain::feat::preferences_actor::app_state_actor::AppStateActor::supervise(
                &root,
                jinn_domain::feat::preferences_actor::app_state_actor::AppStateActorDeps {
                    deps: actor_deps.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

        // App state sync: updates AppState from AppStateUpdated events.
        let _app_state_sync =
            jinn_domain::feat::preferences_actor::app_state_sync_actor::AppStateSyncActor::supervise(&root,
                jinn_domain::feat::preferences_actor::app_state_sync_actor::AppStateSyncActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            ).restart_policy(kameo::supervision::RestartPolicy::Never).spawn().await;

        // ── Domain actors ──────────────────────────────────────────────────

        // LLM streaming actor.
        let _llm = jinn_domain::feat::llm_actor::LlmActor::supervise(
            &root,
            jinn_domain::feat::llm_actor::LlmActorDeps {
                factory: llm_service.clone(),
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;

        // Model discovery actor.
        let _discover = jinn_domain::feat::provider::discover_actor::DiscoverActor::supervise(
            &root,
            jinn_domain::feat::provider::discover_actor::DiscoverActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;

        // Session persistence actor — must spawn before ToolOrchestratorActor so
        // ToolsRegistered subscription is ready when tools register builtins in on_start.
        let token_counter = TiktokenCounter::o200k_base();
        let _session =
            jinn_domain::feat::session::session_actor::SessionPersistenceActor::supervise(
                &root,
                jinn_domain::feat::session::session_actor::SessionPersistenceActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                    counter: token_counter,
                    builtin_registry:
                        jinn_domain::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
                    shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;
        _session.wait_for_startup().await;

        // Tool orchestrator actor.
        let _tools = jinn_domain::feat::tools_actor::ToolOrchestratorActor::supervise(
            &root,
            jinn_domain::feat::tools_actor::ToolOrchestratorActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
                services: services.clone(),
                builtin_filter: None,
                shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;
        _tools.wait_for_startup().await;

        // Register global-scoped plugin tools with the tools actor.
        // Attached-scoped tools are registered per-session when a child session is created.
        {
            use jinn_domain::ToolDefinition;
            use jinn_domain::feat::plugin_system::ToolScopeReexport;
            use jinn_domain::feat::tools_actor::protocol::command::RegisterPluginTools;

            let global_scope_tools: Vec<_> = global_tool_metadata
                .into_iter()
                .filter(|meta| matches!(meta.scope, ToolScopeReexport::Global))
                .collect();

            let mut by_plugin: std::collections::HashMap<String, Vec<_>> =
                std::collections::HashMap::new();
            for meta in global_scope_tools {
                by_plugin
                    .entry(meta.plugin_name.clone())
                    .or_default()
                    .push(meta);
            }
            for (plugin_name, metas) in by_plugin {
                let definitions: Vec<_> = metas
                    .into_iter()
                    .map(|meta| ToolDefinition {
                        name: meta.name,
                        description: meta.description,
                        parameters: meta.parameters,
                        prompt_snippet: None,
                        prompt_guidelines: Vec::new(),
                        server_tool_type: None,
                    })
                    .collect();
                let msg = RegisterPluginTools {
                    plugin_name,
                    target: None,
                    session_id: None,
                    definitions,
                };
                let _closure = jinn_domain::common::bridge::Bridge::publish_closure(msg);
            }
        }

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
        let _web_fetch = WebFetchActor::supervise(
            &root,
            WebFetchActorDeps {
                deps: actor_deps.clone(),
                web_fetcher,
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;

        // Prompt scan actor.
        let _prompt_scan =
            jinn_domain::feat::context::prompt_scan_actor::PromptScanActor::supervise(
                &root,
                jinn_domain::feat::context::prompt_scan_actor::PromptScanActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

        // Context-files scan actor.
        let _context_files =
            jinn_domain::feat::context::context_files_scan_actor::ContextFilesScanActor::supervise(
                &root,
                jinn_domain::feat::context::context_files_scan_actor::ContextFilesScanActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

        // Skills scan actor.
        let _skills_scan =
            jinn_domain::feat::skills::skills_scan_actor::SkillsScanActor::supervise(
                &root,
                jinn_domain::feat::skills::skills_scan_actor::SkillsScanActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

        // Discovery coordinator.
        let _discovery_coordinator =
            jinn_domain::feat::discovery_coordinator::DiscoveryCoordinatorActor::supervise(
                &root,
                jinn_domain::feat::discovery_coordinator::DiscoveryCoordinatorActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

        // Discovery notifier.
        let _discovery_notifier =
            jinn_domain::feat::discovery_notifier::DiscoveryNotifierActor::supervise(
                &root,
                jinn_domain::feat::discovery_notifier::DiscoveryNotifierActorDeps {
                    deps: actor_deps.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

        // Persona scan actor.
        let _persona_scan =
            jinn_domain::feat::persona::persona_scan_actor::PersonaScanActor::supervise(
                &root,
                jinn_domain::feat::persona::persona_scan_actor::PersonaScanActorDeps {
                    deps: actor_deps.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

        // Provider actor.
        let _provider = jinn_domain::feat::provider::provider_actor::ProviderActor::supervise(
            &root,
            jinn_domain::feat::provider::provider_actor::ProviderActorDeps {
                state: state.clone(),
                deps: actor_deps.clone(),
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;

        let _token_count = jinn_domain::feat::token_count_actor::TokenCountActor::supervise(
            &root,
            jinn_domain::feat::token_count_actor::TokenCountActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;

        // Queue actor.
        let _queue = jinn_domain::feat::queue_actor::QueueActor::supervise(
            &root,
            jinn_domain::feat::queue_actor::QueueActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
                counter: token_counter,
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;

        // Context size actor.
        let _context_size =
            jinn_domain::feat::context::context_size_actor::ContextSizeActor::supervise(
                &root,
                jinn_domain::feat::context::context_size_actor::ContextSizeActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                    counter: token_counter,
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

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

            let _snapshot = HistorySnapshotActor::supervise(
                &root,
                HistorySnapshotActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;
        }

        // Compaction worker.
        {
            use jinn_domain::feat::compaction_worker::CompactionWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let _compaction = HistoryWorkerActor::<CompactionWorker>::supervise(
                &root,
                HistoryWorkerActorDeps {
                    deps: actor_deps.clone(),
                    worker: CompactionWorker::new(services.clone(), handle.clone(), state.clone()),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;
        }

        // Compaction trigger actor.
        {
            use jinn_domain::feat::compaction_worker::{
                CompactionTriggerActor, CompactionTriggerActorDeps, CompactionWorker,
            };

            let _trigger = CompactionTriggerActor::supervise(
                &root,
                CompactionTriggerActorDeps {
                    deps: actor_deps.clone(),
                    worker: CompactionWorker::new(services.clone(), handle.clone(), state.clone()),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;
        }

        // Auto-prune worker: read→edit context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::ReadEditAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.read_edit;

            if config.enabled {
                let _worker = HistoryWorkerActor::<ReadEditAutoPruneWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: ReadEditAutoPruneWorker { config },
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
            }
        }

        // Auto-prune worker: edit→read context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::EditReadAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.edit_read;

            if config.enabled {
                let _worker = HistoryWorkerActor::<EditReadAutoPruneWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: EditReadAutoPruneWorker { config },
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
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
                        let _worker = HistoryWorkerActor::<RegexAutoPruneWorker>::supervise(
                            &root,
                            HistoryWorkerActorDeps {
                                deps: actor_deps.clone(),
                                worker,
                            },
                        )
                        .restart_policy(kameo::supervision::RestartPolicy::Never)
                        .spawn()
                        .await;
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

            let config = user_preferences_storage.read().auto_prune.todo;

            if config.enabled {
                let _worker = HistoryWorkerActor::<TodoAutoPruneWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: TodoAutoPruneWorker { config },
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
            }
        }

        // Auto-prune worker: broken-edit context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::BrokenEditAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.broken_edit;

            if config.enabled {
                let _worker = HistoryWorkerActor::<BrokenEditAutoPruneWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: BrokenEditAutoPruneWorker { config },
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
            }
        }

        // Auto-prune worker: double-edit context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::DoubleEditAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.double_edit;

            if config.enabled {
                let _worker = HistoryWorkerActor::<DoubleEditAutoPruneWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: DoubleEditAutoPruneWorker { config },
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
            }
        }

        // Auto-prune worker: consecutive-reads per-file pruning.
        {
            use jinn_domain::feat::auto_prune_worker::ConsecutiveReadsAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.consecutive_reads;

            if config.enabled {
                let _worker = HistoryWorkerActor::<ConsecutiveReadsAutoPruneWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: ConsecutiveReadsAutoPruneWorker { config },
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
            }
        }

        // Shared entry-token cache for history workers.
        let entry_token_cache =
            jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCache::new();

        // HistoryWorkerChatEntryTokenCache eviction actor.
        {
            use jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCacheEvictionActor;
            use jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCacheEvictionActorDeps;

            let _eviction = HistoryWorkerChatEntryTokenCacheEvictionActor::supervise(
                &root,
                HistoryWorkerChatEntryTokenCacheEvictionActorDeps {
                    deps: actor_deps.clone(),
                    cache: entry_token_cache.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;
        }

        // Auto-prune worker: tool-age-window context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::ToolAgeWindowAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.tool_age_window;

            if config.enabled {
                let _worker = HistoryWorkerActor::<ToolAgeWindowAutoPruneWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: ToolAgeWindowAutoPruneWorker { config },
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
            }
        }

        // Auto-prune worker: trivial-assistant context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::TrivialAssistantAutoPruneWorker;
            use jinn_domain::feat::context::strategy::token_estimator::TiktokenCounter;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.trivial_assistant;

            if config.enabled {
                let _worker = HistoryWorkerActor::<TrivialAssistantAutoPruneWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: TrivialAssistantAutoPruneWorker {
                            config,
                            token_cache: entry_token_cache.clone(),
                            counter: TiktokenCounter::o200k_base(),
                        },
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
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
                prefs.auto_prune.anchor_shield
            };

            if shield_config.enabled {
                let _worker = HistoryWorkerActor::<AnchorShieldAutoPruneWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: AnchorShieldAutoPruneWorker {
                            config: shield_config,
                        },
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
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
                let _worker = HistoryWorkerActor::<AnchoredAssistantAutoPruneWorker>::supervise(
                    &root,
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
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await;
            }
        }

        // Sidebar state actor.
        let _sidebar =
            jinn_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActor::supervise(
                &root,
                jinn_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;

        let _plugin_dispatch = PluginDispatchActor::supervise(
            &root,
            PluginDispatchActorDeps {
                deps: actor_deps.clone(),
                services: services.clone(),
                state: state.clone(),
                startup_session_id: state.read().session.active_session_id().to_string(),
                domain_ctx: shared_domain_ctx.clone(),
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;

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
        let _ = ready_rx.to_async().recv().await;

        // Build AppCore with shared state and the bridge.
        let core = AppCore {
            state: state.clone(),
            bridge: services.bridge.clone(),
        };

        (core, services, sync_plugins)
    }
}
