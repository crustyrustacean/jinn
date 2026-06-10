//! Actor wiring - spawns all actors and assembles the actor host.
//!
//! This module encapsulates the one-time startup wiring: creating shared state,
//! spawning each actor via the unified [`spawn`]/[`system_spawn`] functions,
//! building the actor host, starting the forwarding task, and waiting for the
//! actor system to become ready. Called once from `App::dispatch`.
//!
//! # Spawn order
//!
//! 1. Infrastructure actors via [`system_spawn`] (no lifecycle events):
//!    - `system-ready` - counts `ActorStarted`, signals main thread
//! 2. Lifecycle events emitted for both infrastructure actors
//! 3. Init actors via [`spawn`] (self-schedule on startup):
//!    - `env-init` - loads config, resolves API keys
//!    - `provider-init` - builds registry, merges cache, resolves `last_model`
//!    - `preferences` - loads user preferences
//! 4. Domain actors via [`spawn`]:
//!    - All remaining actors

use std::sync::Arc;

use jinn_domain::ApiKeysService;
use jinn_domain::AppState;
use jinn_domain::ConfigStorageService;
use jinn_domain::Event;
use jinn_domain::LlmServiceFactoryService;
use jinn_domain::ProviderRegistryService;
use jinn_domain::Services;
use jinn_domain::SessionStoreService;
use jinn_domain::UserPreferencesStorageService;

use jinn_domain::actor_channel::ActorChannelService;
use jinn_domain::common::actor::protocol::event::{ActorStarted, ActorStarting, AllActorsSpawned};
use jinn_domain::feat::context::strategy::token_estimator::TiktokenCounter;

use jinn_domain::feat::plugin_dispatch::{PluginDispatchActor, PluginDispatchActorDeps};
use jinn_domain::init::env_init_actor::{EnvInitActor, EnvInitActorDeps};
use jinn_domain::init::provider_init_actor::{ProviderInitActor, ProviderInitActorDeps};
use jinn_domain::init::system_ready_actor::{SystemReadyActor, SystemReadyActorDeps};

use jinn_domain::feat::preferences_actor::user_preferences::WebFetchBackend;
use jinn_domain::feat::web_fetch_actor::{WebFetchActor, WebFetchActorDeps};
use jinn_web_fetch::{HttpFetcher, MarkdownExtractor, OutputFormat};

use jinn_domain::{
    ActorCounter, ActorHostService, ActorMessageSink, AppCore, AppMsg, InMemoryActorHost,
    MessageSink, ShutdownTracker, State, spawn, spawn_forwarding_task, system_spawn,
    wait_for_system_ready,
};

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

/// Builds the actor system: spawns all actors and assembles the actor host.
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

    /// Spawn all actors, build the host, start the forwarding task, and wait
    /// for readiness.
    pub fn build(
        self,
    ) -> (
        AppCore,
        Services,
        ActorHostService,
        jinn_plugin::SyncPlugins,
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
        // Body passes `handle` as `&tokio::runtime::Handle`; rebind as a ref.
        let handle: &tokio::runtime::Handle = &handle;
        let bench = self.bench;

        // Create channel first - actors need the sender, but AppCore needs services
        // which needs the actor host which needs actors. Break the cycle by creating
        // the channel independently.
        let (sender, receiver) = kanal::unbounded::<AppMsg>();
        let async_receiver = receiver.to_async();

        // Create the message sink that bridges actor output to AppCore's channel.
        let sink: Arc<dyn MessageSink> = Arc::new(ActorMessageSink::new(sender.clone()));

        // Create the actor counter - incremented by every spawn/system_spawn call.
        let counter = ActorCounter::new();

        // Create the shutdown tracker - shared across all actors for coordinated shutdown.
        let shutdown_tracker = ShutdownTracker::new();

        // Create shared State FIRST - injected into multiple actors.
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

        let plugin_command_dispatcher: jinn_plugin::CommandDispatcher = std::sync::Arc::new({
            let sink = sink.clone();
            move |cmd: jinn_plugin::PluginCommand| {
                crate::plugin_wiring::handle_plugin_command(cmd, &*sink);
            }
        });
        // `DomainNodeContext` is needed by the plugin request handler (for `llm_oneshot`)
        // but it can't be constructed until `services` is assembled (line ~168, after
        // `PluginSystem::new`). Bridge with a `OnceLock` filled in once `services` exists.
        let domain_ctx_cell: std::sync::Arc<
            std::sync::OnceLock<jinn_domain::feat::plugin_dispatch::DomainNodeContext>,
        > = std::sync::Arc::new(std::sync::OnceLock::new());
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

        let plugin_build = jinn_plugin::PluginSystem::build(
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
        };

        // Register global plugin tools with the tools actor.
        if !global_tool_metadata.is_empty() {
            // Group tools by plugin name and send one RegisterPluginTools per plugin.
            let mut by_plugin: std::collections::HashMap<String, Vec<jinn_domain::ToolDefinition>> =
                std::collections::HashMap::new();
            for meta in global_tool_metadata {
                by_plugin
                    .entry(meta.plugin_name.clone())
                    .or_default()
                    .push(jinn_domain::ToolDefinition {
                        name: meta.name,
                        description: meta.description,
                        parameters: meta.parameters,
                        prompt_snippet: None,
                        prompt_guidelines: vec![],
                        server_tool_type: None,
                    });
            }
            for (plugin_name, definitions) in by_plugin {
                services.actor_channel.send_command(
                    jinn_domain::Command::RegisterPluginTools(
                        jinn_domain::feat::tools_actor::protocol::command::RegisterPluginTools {
                            plugin_name,
                            target: None,
                            definitions,
                        },
                    ),
                );
            }
        }

        // Now that `services` + `state` exist, build the shared `DomainNodeContext`
        // and publish it into the `OnceLock` so the plugin request handler can see it.
        let shared_domain_ctx =
            std::sync::Arc::new(jinn_domain::feat::plugin_dispatch::DomainNodeContext::new(
                services.clone(),
                state.clone(),
            ));
        let _ = domain_ctx_cell.set((*shared_domain_ctx).clone());

        // ── Infrastructure actors (no lifecycle events) ──────────────────────

        let mut actors = Vec::new();

        // System-ready actor: counts ActorStarted, signals main thread when done.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        actors.push(system_spawn::<SystemReadyActor>(
            "system-ready",
            sink.clone(),
            handle,
            &counter,
            &shutdown_tracker,
            SystemReadyActorDeps {
                ready_tx,
                counter: counter.clone(),
            },
        ));

        // Emit lifecycle events for the infrastructure actor manually.
        let _ = sink.send_event(Event::ActorStarting(ActorStarting {
            name: "system-ready".to_owned(),
            description: Some("Counts ActorStarted events and signals system ready".to_owned()),
        }));
        let _ = sink.send_event(Event::ActorStarted(ActorStarted {
            name: "system-ready".to_owned(),
            description: Some("Counts ActorStarted events and signals system ready".to_owned()),
        }));

        // ── Init actors (self-schedule Initialize during activate) ────────────

        // Env init: loads providers.toml, resolves API keys, emits EnvironmentLoaded.
        actors.push(spawn::<EnvInitActor>(
            "env-init",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            EnvInitActorDeps {
                services: services.clone(),
            },
        ));

        // Provider init: on EnvironmentLoaded, builds registry, merges cache, resolves last_model.
        actors.push(spawn::<ProviderInitActor>(
            "provider-init",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            ProviderInitActorDeps {
                services: services.clone(),
                state: state.clone(),
            },
        ));

        // Preferences: loads and persists user preferences.
        actors.push(spawn::<
            jinn_domain::feat::preferences_actor::preferences_actor::PreferencesActor,
        >(
            "preferences",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::preferences_actor::preferences_actor::PreferencesActorDeps {
                services: services.clone(),
            },
        ));

        // Preferences state sync: updates AppState from PreferencesUpdated events.
        actors.push(spawn::<
        jinn_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActor,
    >("preferences-sync", &sink, handle, &counter, &shutdown_tracker,
        jinn_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActorDeps {
            services: services.clone(),
            state: state.clone(),
        },
    ));

        // App state actor: persists state changes to state.toml.
        actors.push(spawn::<
            jinn_domain::feat::preferences_actor::app_state_actor::AppStateActor,
        >(
            "app-state",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::preferences_actor::app_state_actor::AppStateActorDeps {
                services: services.clone(),
            },
        ));

        // App state sync: updates AppState from AppStateUpdated events.
        actors.push(spawn::<
            jinn_domain::feat::preferences_actor::app_state_sync_actor::AppStateSyncActor,
        >(
            "app-state-sync",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::preferences_actor::app_state_sync_actor::AppStateSyncActorDeps {
                services: services.clone(),
                state: state.clone(),
            },
        ));

        // ── Domain actors ────────────────────────────────────────────────────

        // LLM streaming actor.
        actors.push(spawn::<jinn_domain::feat::llm_actor::LlmActor>(
            "llm-streaming",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::llm_actor::LlmActorDeps {
                factory: llm_service.clone(),
                services: services.clone(),
                state: state.clone(),
            },
        ));

        // Model discovery actor.
        actors.push(spawn::<
            jinn_domain::feat::provider::discover_actor::DiscoverActor,
        >(
            "llm-provider-listing",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::provider::discover_actor::DiscoverActorDeps {
                services: services.clone(),
                state: state.clone(),
            },
        ));

        // Tool orchestrator actor.
        actors.push(
            spawn::<jinn_domain::feat::tools_actor::ToolOrchestratorActor>(
                "tool-orchestrator",
                &sink,
                handle,
                &counter,
                &shutdown_tracker,
                jinn_domain::feat::tools_actor::ToolOrchestratorActorDeps {
                    services: services.clone(),
                    state: state.clone(),
                    builtin_filter: None,
                    shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
                },
            ),
        );

        // Web fetch actor - reads backend from preferences, constructs fetcher.
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
        actors.push(spawn::<WebFetchActor>(
            "web-fetch",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            WebFetchActorDeps { web_fetcher },
        ));

        // Session persistence actor.
        let token_counter = TiktokenCounter::o200k_base();
        actors.push(spawn::<
            jinn_domain::feat::session::session_actor::SessionPersistenceActor,
        >(
            "session-persistence",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::session::session_actor::SessionPersistenceActorDeps {
                state: state.clone(),
                services: services.clone(),

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
        ));

        // Prompt scan actor.
        actors.push(spawn::<
            jinn_domain::feat::context::prompt_scan_actor::PromptScanActor,
        >(
            "prompt-scan",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::context::prompt_scan_actor::PromptScanActorDeps {
                services: services.clone(),
                state: state.clone(),
            },
        ));

        // Context-files scan actor.
        actors.push(spawn::<
            jinn_domain::feat::context::context_files_scan_actor::ContextFilesScanActor,
        >(
            "context-files-scan",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::context::context_files_scan_actor::ContextFilesScanActorDeps {
                services: services.clone(),
                state: state.clone(),
            },
        ));

        // Skills scan actor.
        actors.push(spawn::<
            jinn_domain::feat::skills::skills_scan_actor::SkillsScanActor,
        >(
            "skills-scan",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::skills::skills_scan_actor::SkillsScanActorDeps {
                services: services.clone(),
                state: state.clone(),
            },
        ));

        // Discovery coordinator — coalesces the three resource-loaded events
        // per session and emits `SessionDiscoverySettled`.
        actors.push(spawn::<
            jinn_domain::feat::discovery_coordinator::DiscoveryCoordinatorActor,
        >(
            "discovery-coordinator",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::discovery_coordinator::DiscoveryCoordinatorActorDeps {
                state: state.clone(),
            },
        ));

        // Discovery notifier — posts a transient chat entry when a session's
        // discovery settles.
        actors.push(spawn::<
            jinn_domain::feat::discovery_notifier::DiscoveryNotifierActor,
        >(
            "discovery-notifier",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::discovery_notifier::DiscoveryNotifierActorDeps,
        ));

        // Persona scan actor.
        actors.push(spawn::<
            jinn_domain::feat::persona::persona_scan_actor::PersonaScanActor,
        >(
            "persona-scan",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::persona::persona_scan_actor::PersonaScanActorDeps {
                services: services.clone(),
            },
        ));

        // Provider actor.
        actors.push(spawn::<
            jinn_domain::feat::provider::provider_actor::ProviderActor,
        >(
            "provider",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::provider::provider_actor::ProviderActorDeps {
                state: state.clone(),
                services: services.clone(),
            },
        ));

        // Token count actor - computes tiktoken counts for chat entries.
        actors.push(
            spawn::<jinn_domain::feat::token_count_actor::TokenCountActor>(
                "token-count",
                &sink,
                handle,
                &counter,
                &shutdown_tracker,
                jinn_domain::feat::token_count_actor::TokenCountActorDeps {
                    state: state.clone(),
                },
            ),
        );

        // Queue actor - dispatches queued turns when sessions become idle.
        actors.push(spawn::<jinn_domain::feat::queue_actor::QueueActor>(
            "queue",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::queue_actor::QueueActorDeps {
                state: state.clone(),
                counter: token_counter,
            },
        ));

        // Context size actor - recalculates context size for the status bar.
        actors.push(spawn::<
            jinn_domain::feat::context::context_size_actor::ContextSizeActor,
        >(
            "context-size",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::context::context_size_actor::ContextSizeActorDeps {
                state: state.clone(),
                counter: token_counter,
            },
        ));

        // ── History mutation workers ────────────────��─────────────────────────
        //
        // To add a new history mutation worker:
        //
        //   1. Implement `HistoryWorker` for your heuristic type
        //      (see `crates/jinn-domain/src/feat/history_worker/worker_trait.rs`).
        //   2. Add a spawn call here:
        //
        //   use jinn_domain::feat::history_worker::{HistoryWorkerActor, HistoryWorkerActorDeps};
        //
        //   actors.push(spawn::<HistoryWorkerActor<MyWorker>>(
        //       "history-worker-my-worker",
        //       &sink, handle, &counter, &shutdown_tracker,
        //       HistoryWorkerActorDeps {
        //           worker: MyWorker::new(),
        //       },
        //   ));
        //

        // ── History snapshot actor ──────────────────────────────────────────
        // Clones history once per HistoryAppended into Arc<[ChatEntry]>,
        // then emits HistorySnapshotReady for all workers to share.
        {
            use jinn_domain::feat::history_worker::snapshot_actor::{
                HistorySnapshotActor, HistorySnapshotActorDeps,
            };

            actors.push(spawn::<HistorySnapshotActor>(
                "history-snapshot",
                &sink,
                handle,
                &counter,
                &shutdown_tracker,
                HistorySnapshotActorDeps {
                    state: state.clone(),
                },
            ));
        }
        // Compaction worker - summarizes conversation history into structured checkpoints.
        {
            use jinn_domain::feat::compaction_worker::CompactionWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            actors.push(spawn::<HistoryWorkerActor<CompactionWorker>>(
                "history-worker-compaction",
                &sink,
                handle,
                &counter,
                &shutdown_tracker,
                HistoryWorkerActorDeps {
                    worker: CompactionWorker::new(services.clone(), handle.clone(), state.clone()),
                },
            ));
        }

        // Compaction trigger actor - handles /compact and /compact-all commands.
        {
            use jinn_domain::feat::compaction_worker::{
                CompactionTriggerActor, CompactionTriggerActorDeps, CompactionWorker,
            };

            actors.push(spawn::<CompactionTriggerActor>(
                "compaction-trigger",
                &sink,
                handle,
                &counter,
                &shutdown_tracker,
                CompactionTriggerActorDeps {
                    worker: CompactionWorker::new(services.clone(), handle.clone(), state.clone()),
                },
            ));
        }

        // Auto-prune worker: read→edit context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::ReadEditAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.read_edit.clone();

            if config.enabled {
                actors.push(spawn::<HistoryWorkerActor<ReadEditAutoPruneWorker>>(
                    "history-worker-auto-prune-read-edit",
                    &sink,
                    handle,
                    &counter,
                    &shutdown_tracker,
                    HistoryWorkerActorDeps {
                        worker: ReadEditAutoPruneWorker { config },
                    },
                ));
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
                actors.push(spawn::<HistoryWorkerActor<EditReadAutoPruneWorker>>(
                    "history-worker-auto-prune-edit-read",
                    &sink,
                    handle,
                    &counter,
                    &shutdown_tracker,
                    HistoryWorkerActorDeps {
                        worker: EditReadAutoPruneWorker { config },
                    },
                ));
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
                        actors.push(spawn::<HistoryWorkerActor<RegexAutoPruneWorker>>(
                            "history-worker-auto-prune-regex",
                            &sink,
                            handle,
                            &counter,
                            &shutdown_tracker,
                            HistoryWorkerActorDeps { worker },
                        ));
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
                actors.push(spawn::<HistoryWorkerActor<TodoAutoPruneWorker>>(
                    "history-worker-auto-prune-todo",
                    &sink,
                    handle,
                    &counter,
                    &shutdown_tracker,
                    HistoryWorkerActorDeps {
                        worker: TodoAutoPruneWorker { config },
                    },
                ));
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
                actors.push(spawn::<HistoryWorkerActor<BrokenEditAutoPruneWorker>>(
                    "history-worker-auto-prune-broken-edit",
                    &sink,
                    handle,
                    &counter,
                    &shutdown_tracker,
                    HistoryWorkerActorDeps {
                        worker: BrokenEditAutoPruneWorker { config },
                    },
                ));
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
                actors.push(spawn::<HistoryWorkerActor<DoubleEditAutoPruneWorker>>(
                    "history-worker-auto-prune-double-edit",
                    &sink,
                    handle,
                    &counter,
                    &shutdown_tracker,
                    HistoryWorkerActorDeps {
                        worker: DoubleEditAutoPruneWorker { config },
                    },
                ));
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
                actors.push(
                    spawn::<HistoryWorkerActor<ConsecutiveReadsAutoPruneWorker>>(
                        "history-worker-auto-prune-consecutive-reads",
                        &sink,
                        handle,
                        &counter,
                        &shutdown_tracker,
                        HistoryWorkerActorDeps {
                            worker: ConsecutiveReadsAutoPruneWorker { config },
                        },
                    ),
                );
            }
        }

        // Shared entry-token cache for history workers.
        //
        // Constructed once, cloned into any worker (or peer actor) that needs
        // per-entry token counts. Eviction is handled by
        // HistoryWorkerChatEntryTokenCacheEvictionActor below.
        let entry_token_cache =
            jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCache::new();

        // HistoryWorkerChatEntryTokenCache eviction actor — single instance,
        // owns session lifecycle.
        actors.push(spawn::<
        jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCacheEvictionActor,
    >(
        "history-worker-token-cache-eviction",
        &sink,
        handle,
        &counter,
        &shutdown_tracker,
        jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCacheEvictionActorDeps {
            cache: entry_token_cache.clone(),
        },
    ));
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
                actors.push(spawn::<HistoryWorkerActor<ToolAgeWindowAutoPruneWorker>>(
                    "history-worker-auto-prune-tool-age-window",
                    &sink,
                    handle,
                    &counter,
                    &shutdown_tracker,
                    HistoryWorkerActorDeps {
                        worker: ToolAgeWindowAutoPruneWorker { config },
                    },
                ));
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
                actors.push(
                    spawn::<HistoryWorkerActor<TrivialAssistantAutoPruneWorker>>(
                        "history-worker-auto-prune-trivial-assistant",
                        &sink,
                        handle,
                        &counter,
                        &shutdown_tracker,
                        HistoryWorkerActorDeps {
                            worker: TrivialAssistantAutoPruneWorker {
                                config,
                                token_cache: entry_token_cache.clone(),
                                counter: TiktokenCounter::o200k_base(),
                            },
                        },
                    ),
                );
            }
        }

        // Auto-prune worker: anchored-assistant context pruning.
        // Prunes large (>80 token) Assistant entries whose index distance to the
        // nearest anchor entry (first index, last index, or any User entry)
        // exceeds a configurable radius.
        {
            use jinn_domain::feat::auto_prune_worker::AnchoredAssistantAutoPruneWorker;
            use jinn_domain::feat::context::strategy::token_estimator::TiktokenCounter;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let (config, trivial_max_tokens) = {
                let prefs = user_preferences_storage.read();
                let cfg = prefs.auto_prune.anchored_assistant.clone();
                let max_tokens = prefs.auto_prune.trivial_assistant.max_tokens as u32;
                (cfg, max_tokens)
            };

            if config.enabled {
                actors.push(
                    spawn::<HistoryWorkerActor<AnchoredAssistantAutoPruneWorker>>(
                        "history-worker-auto-prune-anchored-assistant",
                        &sink,
                        handle,
                        &counter,
                        &shutdown_tracker,
                        HistoryWorkerActorDeps {
                            worker: AnchoredAssistantAutoPruneWorker {
                                config,
                                min_candidate_tokens: trivial_max_tokens + 1,
                                token_cache: entry_token_cache.clone(),
                                counter: TiktokenCounter::o200k_base(),
                            },
                        },
                    ),
                );
            }
        }
        // Sidebar state actor - keeps sidebar cursor in sync after session removal.
        actors.push(spawn::<
            jinn_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActor,
        >(
            "sidebar-state",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            jinn_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActorDeps {
                state: state.clone(),
            },
        ));

        // ── Plugin dispatch actor (replaces plugin_lifecycle + workflow_controller) ─
        actors.push(spawn::<PluginDispatchActor>(
            "plugin-dispatch",
            &sink,
            handle,
            &counter,
            &shutdown_tracker,
            PluginDispatchActorDeps {
                services: services.clone(),
                state: state.clone(),
                startup_session_id: state.read().session.active_session_id().to_string(),
                domain_ctx: shared_domain_ctx.clone(),
            },
        ));

        // ── Bench actor (conditional) ─────────────────────────────────────────
        if let Some(b) = bench {
            actors.push(spawn::<jinn_bench::bench_actor::BenchActor>(
                "bench",
                &sink,
                handle,
                &counter,
                &shutdown_tracker,
                jinn_bench::bench_actor::BenchActorDeps {
                    state: state.clone(),
                    csv_path: Some(b.csv_path.clone()),
                    plan: Some(b.plan),
                },
            ));
        }

        // Spawn the async forwarding task - continuously drains AppMsg channel → actor host.
        let actor_host_service = ActorHostService::new(Arc::new(
            InMemoryActorHost::from_actors_with_handle(actors, handle.clone(), shutdown_tracker),
        ));
        spawn_forwarding_task(async_receiver, actor_host_service.clone(), handle);

        // Signal that all actors have been spawned.
        // SystemReadyActor waits for this before checking its count.
        let _ = sink.send_event(Event::AllActorsSpawned(AllActorsSpawned));

        // Wait for the actor system to become ready (3-second timeout).
        wait_for_system_ready(ready_rx, handle);

        // Build AppCore with shared state and sender only.
        let core = AppCore {
            state: state.clone(),
            sender: sender.clone(),
        };

        // Trigger initial persona scan.
        let _ = sink.send_command(jinn_domain::Command::RescanPersonas(
            jinn_domain::feat::context::protocol::command::RescanPersonas,
        ));

        (core, services, actor_host_service, sync_plugins)
    }
}
