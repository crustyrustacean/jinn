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

/// Spawn a kameo actor and announce its lifecycle on the bus.
///
/// Publishes `ActorStarting` before the spawn future resolves and
/// `ActorStarted` after, so the dashboard can display the lifecycle.
macro_rules! spawn_tracked {
    ($bus:expr, $name:expr, $desc:expr, $spawn:expr) => {{
        let __bus = $bus.actor_ref();
        let __name: &str = $name;
        let __desc: &str = $desc;
        let _ = __bus
            .tell(kameo_actors::message_bus::Publish(
                jinn_domain::common::actor::protocol::event::ActorStarting {
                    name: __name.to_string(),
                    description: Some(__desc.to_string()),
                },
            ))
            .await;
        let __actor = $spawn;
        let _ = __bus
            .tell(kameo_actors::message_bus::Publish(
                jinn_domain::common::actor::protocol::event::ActorStarted {
                    name: __name.to_string(),
                    description: Some(__desc.to_string()),
                },
            ))
            .await;
        __actor
    }};
}

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

/// Filters discovered plugins down to attachable ones and maps them into
/// [`DiscoveredPlugin`] entries for the plugin picker.
///
/// Global plugins are loaded at startup and cannot be attached per-session,
/// so they're excluded from the picker that issues `AttachPlugin`.
fn attachable_discovered_plugins(
    plugins: Vec<jinn_plugin::PluginMeta>,
) -> Vec<jinn_domain::common::app_state::DiscoveredPlugin> {
    plugins
        .into_iter()
        .filter(|p| p.kind == jinn_plugin::PluginKind::Attachable)
        .map(|p| jinn_domain::common::app_state::DiscoveredPlugin {
            name: p.name,
            description: p.description,
        })
        .collect()
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
        jinn_plugin::SyncPlugins,
        Option<kanal::AsyncReceiver<jinn_domain::feat::discord::BridgeEvent>>,
        kanal::Sender<jinn_domain::feat::dashboard::status_actor::DiscordStatusUpdate>,
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

        let plugin_command_dispatcher: jinn_plugin::CommandDispatcher =
            crate::plugin_wiring::build_command_dispatcher(bridge.clone());
        let handler_cell = domain_ctx_cell.clone();
        let plugin_request_handler: jinn_plugin::RequestHandler = std::sync::Arc::new({
            move |name: &str,
                  data: &serde_json::Value,
                  cancel: Option<tokio_util::sync::CancellationToken>| {
                let cell = handler_cell.clone();
                let name = name.to_string();
                let data = data.clone();
                std::boxed::Box::pin(async move {
                    match cell.get() {
                        Some(ctx) => {
                            crate::plugin_wiring::handle_plugin_request(
                                &name,
                                &data,
                                ctx,
                                cancel.as_ref(),
                            )
                            .await
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

        // Store discovered plugin metadata in state for the plugin picker.
        // Only attachable plugins are exposed — global plugins are loaded at
        // startup and cannot be attached per-session.
        {
            let plugins =
                jinn_plugin::discover_plugins(&paths.plugins_dir(), &paths.system_plugins_dir());
            tracing::info!(count = plugins.len(), "discovered plugins");
            state.write().discovered_plugins = attachable_discovered_plugins(plugins);
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
                jinn_domain::feat::plugin_dispatch::SessionPluginRegistryService::new(
                    std::sync::Arc::new(async_plugins.clone())
                        as std::sync::Arc<
                            dyn jinn_domain::feat::plugin_dispatch::SessionPluginRegistry,
                        >,
                ),
            tempdir: None,
            bus,
            bridge: bridge.clone(),
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

        // ── Discord status actor ─────────────────────────────────────────
        // Always spawned FIRST — subscribes to lifecycle events before any
        // other actor fires them, so the dashboard captures every actor.
        // The kanal sender feeds the discord connection sub-state from the
        // gateway task (best-effort, ignored when discord is disabled).
        let (discord_status_tx, discord_status_rx) =
            kanal::unbounded::<jinn_domain::feat::dashboard::status_actor::DiscordStatusUpdate>();
        let _discord_status =
            jinn_domain::feat::dashboard::status_actor::DiscordStatusActor::supervise(
                &root,
                jinn_domain::feat::dashboard::status_actor::DiscordStatusActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                    status_rx: discord_status_rx.to_async(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await;
        // Wait for the dashboard actor's subscriptions to be fully wired
        // before spawning any other actors. Without this, the bus events
        // (ActorStarting/ActorStarted) from subsequently spawned actors
        // can be missed — leaving their dashboard entries stuck on
        // "Starting" because ActorStarted was never received.
        _discord_status.wait_for_startup().await;

        // ── Infrastructure actors ──────────────────────────────────────────

        // System-ready actor: signals main thread when all actors started.
        let (ready_tx, ready_rx) = kanal::unbounded::<()>();
        let _system_ready = spawn_tracked!(
            &services.bus,
            "system-ready",
            "SystemReadyActor",
            SystemReadyActor::supervise(
                &root,
                SystemReadyActorDeps {
                    deps: actor_deps.clone(),
                    ready_tx,
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // ── Init actors ────────────────────────────────────────────────────

        // Env init: registers in actor registry, defers config loading to GetEnvironmentConfig ask.
        let env_init = spawn_tracked!(
            &services.bus,
            "env-init",
            "EnvInitActor",
            EnvInitActor::supervise(
                &root,
                EnvInitActorDeps {
                    deps: actor_deps.clone(),
                    registry_name: Some("env-init"),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );
        env_init.wait_for_startup().await;
        // Provider init: on EnvironmentLoaded, builds registry, merges cache, resolves last_model.
        let _provider_init = spawn_tracked!(
            &services.bus,
            "provider-init",
            "ProviderInitActor",
            ProviderInitActor::supervise(
                &root,
                ProviderInitActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Preferences: loads and persists user preferences.
        let _preferences =
            spawn_tracked!(&services.bus, "preferences", "PreferencesActor",
jinn_domain::feat::preferences_actor::preferences_actor::PreferencesActor::supervise(
                    &root,
                    jinn_domain::feat::preferences_actor::preferences_actor::PreferencesActorDeps {
                        deps: actor_deps.clone(),
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await
        );

        // Preferences state sync: updates AppState from PreferencesUpdated events.
        let _preferences_sync =
            spawn_tracked!(&services.bus, "preferences-state-sync", "PreferencesStateSyncActor",
jinn_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActor::supervise(&root,
                    jinn_domain::feat::preferences_actor::preferences_state_sync_actor::PreferencesStateSyncActorDeps {
                        deps: actor_deps.clone(),
                        state: state.clone(),
                    },
                ).restart_policy(kameo::supervision::RestartPolicy::Never).spawn().await
        );

        // App state actor: persists state changes to state.toml.
        let _app_state = spawn_tracked!(
            &services.bus,
            "app-state",
            "AppStateActor",
            jinn_domain::feat::preferences_actor::app_state_actor::AppStateActor::supervise(
                &root,
                jinn_domain::feat::preferences_actor::app_state_actor::AppStateActorDeps {
                    deps: actor_deps.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // App state sync: updates AppState from AppStateUpdated events.
        let _app_state_sync =
            spawn_tracked!(&services.bus, "app-state-sync", "AppStateSyncActor",
jinn_domain::feat::preferences_actor::app_state_sync_actor::AppStateSyncActor::supervise(&root,
                    jinn_domain::feat::preferences_actor::app_state_sync_actor::AppStateSyncActorDeps {
                        deps: actor_deps.clone(),
                        state: state.clone(),
                    },
                ).restart_policy(kameo::supervision::RestartPolicy::Never).spawn().await
        );

        // Quake bar: owns the command log; sole subscriber of SubmitQuakeBarCommand.
        let _quake_bar = spawn_tracked!(
            &services.bus,
            "quake-bar",
            "QuakeBarActor",
            jinn_domain::feat::quake_bar::quake_bar_actor::QuakeBarActor::supervise(
                &root,
                jinn_domain::feat::quake_bar::quake_bar_actor::QuakeBarActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // ── Domain actors ──────────────────────────────────────────────────

        // LLM streaming actor.
        let _llm = spawn_tracked!(
            &services.bus,
            "llm",
            "LlmActor",
            jinn_domain::feat::llm_actor::LlmActor::supervise(
                &root,
                jinn_domain::feat::llm_actor::LlmActorDeps {
                    factory: llm_service.clone(),
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Model discovery actor.
        let _discover = spawn_tracked!(
            &services.bus,
            "discover",
            "DiscoverActor",
            jinn_domain::feat::provider::discover_actor::DiscoverActor::supervise(
                &root,
                jinn_domain::feat::provider::discover_actor::DiscoverActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Session persistence actor — must spawn before ToolOrchestratorActor so
        // ToolsRegistered subscription is ready when tools register builtins in on_start.
        //
        // Unbounded mailbox: the session actor is the single sink for every streaming
        // event (StreamToken, StreamCompleted, ToolBatchCompleted, …) from a provider
        // burst. The default bounded(64) mailbox can momentarily fill at the [DONE]
        // peak of a large reasoning turn, and because the bus uses BestEffort
        // (try_send) delivery, the terminal `StreamCompleted(ToolUse)` gets silently
        // dropped on `MailboxFull` — permanently wedging the session (the phase never
        // advances out of Streaming). An unbounded mailbox means try_send always
        // succeeds, so the critical control message can never be dropped. There is no
        // deadlock risk: nothing downstream awaits the session actor's mailbox
        // capacity (publishers use fire-and-forget tell under BestEffort).
        let token_counter = TiktokenCounter::o200k_base();
        // Shared entry-token cache for history workers — created here so the
        // session actor's accumulation gate can read it too.
        let entry_token_cache =
            jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCache::new();
        let _session =
            jinn_domain::feat::session::session_actor::SessionPersistenceActor::supervise(
                &root,
                jinn_domain::feat::session::session_actor::SessionPersistenceActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                    counter: token_counter,
                    token_cache: entry_token_cache.clone(),
                    builtin_registry:
                        jinn_domain::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
                    shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn_with_mailbox(kameo::mailbox::unbounded())
            .await;
        _session.wait_for_startup().await;

        // Tool orchestrator actor.
        let _tools = spawn_tracked!(
            &services.bus,
            "tool-orchestrator",
            "ToolOrchestratorActor",
            jinn_domain::feat::tools_actor::ToolOrchestratorActor::supervise(
                &root,
                jinn_domain::feat::tools_actor::ToolOrchestratorActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                    services: services.clone(),
                    builtin_filter: None,
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );
        _tools.wait_for_startup().await;

        // Register global-scoped plugin tools with the tools actor.
        // Global-scope plugin tools: registered globally (execution + visibility).
        {
            use jinn_domain::ToolDefinition;
            use jinn_domain::feat::tools_actor::protocol::command::RegisterPluginTools;
            use jinn_plugin::ToolScopeReexport;

            let global_scope_tools: Vec<_> = global_tool_metadata
                .clone()
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
                    execution_only: false,
                };
                let closure = jinn_domain::common::bridge::Bridge::publish_closure(msg);
                if let Err(e) = bridge.clone().send(closure) {
                    tracing::warn!(error = %e, "failed to send global plugin tool registration");
                }
            }
        }

        // Attached-scope plugin tools: handlers are loaded globally at startup,
        // and their definitions are cataloged for spawn-time resolution.
        // Register execution-only here (no ToolsRegistered event, so nothing
        // lands in global_tool_definitions), and mirror the same definitions
        // into `attachable_tool_catalog` so `create_child_session` can resolve
        // named tools for a spawned child. The origin session never sees these
        // tools (visibility is granted only to the child by copying from the
        // catalog into `session_tool_definitions[child]`).
        // but their VISIBILITY must stay per-session (registered on attach
        // via the dispatch actor). Register execution-only here (no
        // ToolsRegistered event, so nothing lands in global_tool_definitions).
        {
            use jinn_domain::ToolDefinition;
            use jinn_domain::feat::tools_actor::protocol::command::RegisterPluginTools;
            use jinn_plugin::ToolScopeReexport;

            let attached_scope_tools: Vec<_> = global_tool_metadata
                .into_iter()
                .filter(|meta| matches!(meta.scope, ToolScopeReexport::Attached))
                .collect();

            let mut by_plugin: std::collections::HashMap<String, Vec<_>> =
                std::collections::HashMap::new();
            for meta in attached_scope_tools {
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

                // Mirror definitions into the attachable catalog so spawned
                // child sessions can resolve named tools via `create_child_session`.
                {
                    let mut s = state.write();
                    for def in &definitions {
                        s.context
                            .attachable_tool_catalog
                            .insert(def.name.clone(), def.clone());
                    }
                }

                let msg = RegisterPluginTools {
                    plugin_name,
                    target: None,
                    session_id: None,
                    definitions,
                    execution_only: true,
                };
                let closure = jinn_domain::common::bridge::Bridge::publish_closure(msg);
                if let Err(e) = bridge.clone().send(closure) {
                    tracing::warn!(error = %e, "failed to send attached plugin tool registration");
                }
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
        let _web_fetch = spawn_tracked!(
            &services.bus,
            "web-fetch",
            "WebFetchActor",
            WebFetchActor::supervise(
                &root,
                WebFetchActorDeps {
                    deps: actor_deps.clone(),
                    web_fetcher,
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Prompt scan actor.
        let _prompt_scan = spawn_tracked!(
            &services.bus,
            "prompt-scan",
            "PromptScanActor",
            jinn_domain::feat::context::prompt_scan_actor::PromptScanActor::supervise(
                &root,
                jinn_domain::feat::context::prompt_scan_actor::PromptScanActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Context-files scan actor.
        let _context_files = spawn_tracked!(
            &services.bus,
            "context-files-scan",
            "ContextFilesScanActor",
            jinn_domain::feat::context::context_files_scan_actor::ContextFilesScanActor::supervise(
                &root,
                jinn_domain::feat::context::context_files_scan_actor::ContextFilesScanActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Skills scan actor.
        let _skills_scan = spawn_tracked!(
            &services.bus,
            "skills-scan",
            "SkillsScanActor",
            jinn_domain::feat::skills::skills_scan_actor::SkillsScanActor::supervise(
                &root,
                jinn_domain::feat::skills::skills_scan_actor::SkillsScanActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Discovery coordinator.
        let _discovery_coordinator = spawn_tracked!(
            &services.bus,
            "discovery-coordinator",
            "DiscoveryCoordinatorActor",
            jinn_domain::feat::discovery_coordinator::DiscoveryCoordinatorActor::supervise(
                &root,
                jinn_domain::feat::discovery_coordinator::DiscoveryCoordinatorActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Discovery notifier.
        let _discovery_notifier = spawn_tracked!(
            &services.bus,
            "discovery-notifier",
            "DiscoveryNotifierActor",
            jinn_domain::feat::discovery_notifier::DiscoveryNotifierActor::supervise(
                &root,
                jinn_domain::feat::discovery_notifier::DiscoveryNotifierActorDeps {
                    deps: actor_deps.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Persona scan actor.
        let _persona_scan = spawn_tracked!(
            &services.bus,
            "persona-scan",
            "PersonaScanActor",
            jinn_domain::feat::persona::persona_scan_actor::PersonaScanActor::supervise(
                &root,
                jinn_domain::feat::persona::persona_scan_actor::PersonaScanActorDeps {
                    deps: actor_deps.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Provider actor.
        let _provider = spawn_tracked!(
            &services.bus,
            "provider",
            "ProviderActor",
            jinn_domain::feat::provider::provider_actor::ProviderActor::supervise(
                &root,
                jinn_domain::feat::provider::provider_actor::ProviderActorDeps {
                    state: state.clone(),
                    deps: actor_deps.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        let _token_count = spawn_tracked!(
            &services.bus,
            "token-count",
            "TokenCountActor",
            jinn_domain::feat::token_count_actor::TokenCountActor::supervise(
                &root,
                jinn_domain::feat::token_count_actor::TokenCountActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Stall watchdog — detects hung sessions and retries/cancels them.
        let _stall_watchdog = spawn_tracked!(
            &services.bus,
            "stall-watchdog",
            "StallWatchdogActor",
            jinn_domain::feat::stall_watchdog_actor::StallWatchdogActor::supervise(
                &root,
                jinn_domain::feat::stall_watchdog_actor::StallWatchdogActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Queue actor.
        let _queue = spawn_tracked!(
            &services.bus,
            "queue",
            "QueueActor",
            jinn_domain::feat::queue_actor::QueueActor::supervise(
                &root,
                jinn_domain::feat::queue_actor::QueueActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                    counter: token_counter,
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        // Context size actor.
        let _context_size = spawn_tracked!(
            &services.bus,
            "context-size",
            "ContextSizeActor",
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
            .await
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

            let _snapshot = spawn_tracked!(
                &services.bus,
                "history-snapshot",
                "HistorySnapshotActor",
                HistorySnapshotActor::supervise(
                    &root,
                    HistorySnapshotActorDeps {
                        deps: actor_deps.clone(),
                        state: state.clone(),
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await
            );
        }

        // Compaction worker.
        {
            use jinn_domain::feat::compaction_worker::CompactionWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let _compaction = spawn_tracked!(
                &services.bus,
                "history-compaction",
                "HistoryWorker<CompactionWorker>",
                HistoryWorkerActor::<CompactionWorker>::supervise(
                    &root,
                    HistoryWorkerActorDeps {
                        deps: actor_deps.clone(),
                        worker: CompactionWorker::new(
                            services.clone(),
                            handle.clone(),
                            state.clone()
                        ),
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await
            );
        }

        // Compaction trigger actor.
        {
            use jinn_domain::feat::compaction_worker::{
                CompactionTriggerActor, CompactionTriggerActorDeps, CompactionWorker,
            };

            let _trigger = spawn_tracked!(
                &services.bus,
                "compaction-trigger",
                "CompactionTriggerActor",
                CompactionTriggerActor::supervise(
                    &root,
                    CompactionTriggerActorDeps {
                        deps: actor_deps.clone(),
                        worker: CompactionWorker::new(
                            services.clone(),
                            handle.clone(),
                            state.clone()
                        ),
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await
            );
        }

        // Auto-prune worker: read→edit context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::ReadEditAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.read_edit;

            if config.enabled {
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-read-edit",
                    "HistoryWorker<ReadEditAutoPruneWorker>",
                    HistoryWorkerActor::<ReadEditAutoPruneWorker>::supervise(
                        &root,
                        HistoryWorkerActorDeps {
                            deps: actor_deps.clone(),
                            worker: ReadEditAutoPruneWorker { config },
                        },
                    )
                    .restart_policy(kameo::supervision::RestartPolicy::Never)
                    .spawn()
                    .await
                );
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
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-edit-read",
                    "HistoryWorker<EditReadAutoPruneWorker>",
                    HistoryWorkerActor::<EditReadAutoPruneWorker>::supervise(
                        &root,
                        HistoryWorkerActorDeps {
                            deps: actor_deps.clone(),
                            worker: EditReadAutoPruneWorker { config },
                        },
                    )
                    .restart_policy(kameo::supervision::RestartPolicy::Never)
                    .spawn()
                    .await
                );
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
                        let _worker = spawn_tracked!(
                            &services.bus,
                            "history-regex",
                            "HistoryWorker<RegexAutoPruneWorker>",
                            HistoryWorkerActor::<RegexAutoPruneWorker>::supervise(
                                &root,
                                HistoryWorkerActorDeps {
                                    deps: actor_deps.clone(),
                                    worker,
                                },
                            )
                            .restart_policy(kameo::supervision::RestartPolicy::Never)
                            .spawn()
                            .await
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

            let config = user_preferences_storage.read().auto_prune.todo;

            if config.enabled {
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-todo",
                    "HistoryWorker<TodoAutoPruneWorker>",
                    HistoryWorkerActor::<TodoAutoPruneWorker>::supervise(
                        &root,
                        HistoryWorkerActorDeps {
                            deps: actor_deps.clone(),
                            worker: TodoAutoPruneWorker { config },
                        },
                    )
                    .restart_policy(kameo::supervision::RestartPolicy::Never)
                    .spawn()
                    .await
                );
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
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-broken-edit",
                    "HistoryWorker<BrokenEditAutoPruneWorker>",
                    HistoryWorkerActor::<BrokenEditAutoPruneWorker>::supervise(
                        &root,
                        HistoryWorkerActorDeps {
                            deps: actor_deps.clone(),
                            worker: BrokenEditAutoPruneWorker { config },
                        },
                    )
                    .restart_policy(kameo::supervision::RestartPolicy::Never)
                    .spawn()
                    .await
                );
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
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-double-edit",
                    "HistoryWorker<DoubleEditAutoPruneWorker>",
                    HistoryWorkerActor::<DoubleEditAutoPruneWorker>::supervise(
                        &root,
                        HistoryWorkerActorDeps {
                            deps: actor_deps.clone(),
                            worker: DoubleEditAutoPruneWorker { config },
                        },
                    )
                    .restart_policy(kameo::supervision::RestartPolicy::Never)
                    .spawn()
                    .await
                );
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
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-consecutive-reads",
                    "HistoryWorker<ConsecutiveReadsAutoPruneWorker>",
                    HistoryWorkerActor::<ConsecutiveReadsAutoPruneWorker>::supervise(
                        &root,
                        HistoryWorkerActorDeps {
                            deps: actor_deps.clone(),
                            worker: ConsecutiveReadsAutoPruneWorker { config },
                        },
                    )
                    .restart_policy(kameo::supervision::RestartPolicy::Never)
                    .spawn()
                    .await
                );
            }
        }

        // HistoryWorkerChatEntryTokenCache eviction actor.

        // HistoryWorkerChatEntryTokenCache eviction actor.
        {
            use jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCacheEvictionActor;
            use jinn_domain::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCacheEvictionActorDeps;

            let _eviction = spawn_tracked!(
                &services.bus,
                "history-worker-chat-entry-token-cache-eviction",
                "HistoryWorkerChatEntryTokenCacheEvictionActor",
                HistoryWorkerChatEntryTokenCacheEvictionActor::supervise(
                    &root,
                    HistoryWorkerChatEntryTokenCacheEvictionActorDeps {
                        deps: actor_deps.clone(),
                        cache: entry_token_cache.clone(),
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await
            );
        }

        // Auto-prune worker: tool-age-window context pruning.
        {
            use jinn_domain::feat::auto_prune_worker::ToolAgeWindowAutoPruneWorker;
            use jinn_domain::feat::history_worker::actor::{
                HistoryWorkerActor, HistoryWorkerActorDeps,
            };

            let config = user_preferences_storage.read().auto_prune.tool_age_window;

            if config.enabled {
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-tool-age-window",
                    "HistoryWorker<ToolAgeWindowAutoPruneWorker>",
                    HistoryWorkerActor::<ToolAgeWindowAutoPruneWorker>::supervise(
                        &root,
                        HistoryWorkerActorDeps {
                            deps: actor_deps.clone(),
                            worker: ToolAgeWindowAutoPruneWorker { config },
                        },
                    )
                    .restart_policy(kameo::supervision::RestartPolicy::Never)
                    .spawn()
                    .await
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

            let config = user_preferences_storage.read().auto_prune.trivial_assistant;

            if config.enabled {
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-trivial-assistant",
                    "HistoryWorker<TrivialAssistantAutoPruneWorker>",
                    HistoryWorkerActor::<TrivialAssistantAutoPruneWorker>::supervise(
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
                    .await
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
                prefs.auto_prune.anchor_shield
            };

            if shield_config.enabled {
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-anchor-shield",
                    "HistoryWorker<AnchorShieldAutoPruneWorker>",
                    HistoryWorkerActor::<AnchorShieldAutoPruneWorker>::supervise(
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
                    .await
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
                let _worker = spawn_tracked!(
                    &services.bus,
                    "history-anchored-assistant",
                    "HistoryWorker<AnchoredAssistantAutoPruneWorker>",
                    HistoryWorkerActor::<AnchoredAssistantAutoPruneWorker>::supervise(
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
                    .await
                );
            }
        }

        // Sidebar state actor.
        let _sidebar = spawn_tracked!(
            &services.bus,
            "sidebar-state",
            "SidebarStateActor",
            jinn_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActor::supervise(
                &root,
                jinn_domain::feat::ui::sidebar::sidebar_state_actor::SidebarStateActorDeps {
                    deps: actor_deps.clone(),
                    state: state.clone(),
                },
            )
            .restart_policy(kameo::supervision::RestartPolicy::Never)
            .spawn()
            .await
        );

        let _plugin_dispatch = spawn_tracked!(
            &services.bus,
            "plugin-dispatch",
            "PluginDispatchActor",
            PluginDispatchActor::supervise(
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
            .await
        );

        // ── Discord bot bridge actor ───────────────────────────────────────
        // Conditionally spawned when `[discord] enabled = true` in jinn.toml.
        // The bridge forwards bus events (turn-finished, setup-completed) onto a
        // bounded channel that the poise gateway task drains. The gateway itself
        // is spawned AFTER build() returns in app.rs (so it never blocks readiness).
        let discord_cfg = user_preferences_storage.read().discord.clone();
        let discord_bridge_rx = if discord_cfg.enabled {
            let (tx, rx) = kanal::bounded::<jinn_domain::feat::discord::BridgeEvent>(64);
            let async_rx = rx.to_async();
            let _discord_bridge = spawn_tracked!(
                &services.bus,
                "discord-bridge",
                "DiscordBridgeActor",
                jinn_domain::feat::discord::DiscordBridgeActor::supervise(
                    &root,
                    jinn_domain::feat::discord::DiscordBridgeActorDeps {
                        deps: actor_deps.clone(),
                        tx,
                    },
                )
                .restart_policy(kameo::supervision::RestartPolicy::Never)
                .spawn()
                .await
            );
            Some(async_rx)
        } else {
            None
        };
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

        (
            core,
            services,
            sync_plugins,
            discord_bridge_rx,
            discord_status_tx,
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jinn_plugin::{PluginKind, PluginMeta};
    use std::path::PathBuf;

    fn meta(name: &str, kind: PluginKind) -> PluginMeta {
        PluginMeta {
            name: name.to_owned(),
            path: PathBuf::new(),
            description: Some(format!("desc for {name}")),
            kind,
        }
    }

    #[test]
    fn global_plugin_excluded_from_discovered_plugins() {
        // Given a mix of one global and one attachable plugin.
        let plugins = vec![
            meta("welcome", PluginKind::Global),
            meta("judge", PluginKind::Attachable),
        ];

        // When filtering to attachable plugins.
        let result = attachable_discovered_plugins(plugins);

        // Then only the attachable plugin is kept.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "judge");
    }

    #[test]
    fn all_attachable_plugins_kept_global_dropped() {
        // Given two attachable plugins and one global.
        let plugins = vec![
            meta("judge", PluginKind::Attachable),
            meta("consensus", PluginKind::Attachable),
            meta("welcome", PluginKind::Global),
        ];

        // When filtering to attachable plugins.
        let result = attachable_discovered_plugins(plugins);

        // Then both attachable plugins are kept and the global is dropped.
        let names: Vec<&str> = result.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["judge", "consensus"]);
    }

    #[test]
    fn empty_when_only_global_plugins_discovered() {
        // Given only global plugins.
        let plugins = vec![
            meta("welcome", PluginKind::Global),
            meta("prompt_enrichment", PluginKind::Global),
        ];

        // When filtering to attachable plugins.
        let result = attachable_discovered_plugins(plugins);

        // Then nothing remains to attach.
        assert!(result.is_empty());
    }
}
