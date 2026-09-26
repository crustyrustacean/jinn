//! Actor wiring — spawns all actors on the trouper runtime.
//!
//! This module encapsulates the one-time startup wiring: creating shared state,
//! spawning each actor via trouper, building the bus and bridge, and waiting
//! for the actor system to become ready. Called once from `App::dispatch`.
//!
//! # Spawn order
//!
//! 1. Infrastructure actors (system-ready, env-init).
//! 2. Init actors (provider-init, preferences, scan actors).
//! 3. Domain actors (session, tools, history workers, etc.).
//!
//! EnvInitActor is spawned first so dependent actors can pull config from it
//! via `ask()` on its path.

use jinn_domain::ApiKeysService;
use jinn_domain::AppState;
use jinn_domain::ConfigStorageService;
use jinn_domain::LlmServiceFactoryService;
use jinn_domain::ProviderRegistryService;
use jinn_domain::Services;
use jinn_domain::SessionStoreService;
use jinn_provider_selection;
use jinn_quake_bar;
use jinn_slices;

use jinn_domain::common::actor_deps::ActorDeps;
use jinn_llm_support::token_estimator::TiktokenCounter;

use jinn_domain::{AppCore, State};

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
    /// The configuration layer: the live `jinn.toml` every consumer reads.
    pub config: jinn_config::ConfigLayer,
    /// App state storage service.
    pub app_state_storage: jinn_preferences_config::AppStateStorageService,
    /// Application paths.
    pub paths: jinn_domain::AppPaths,
    /// Dump directory for provider request debugging. `None` disables.
    pub dump_requests: Option<std::path::PathBuf>,
    /// The compaction system prompt loaded from the prompts directory at
    /// startup (the compaction worker consumes it on each compaction).
    pub compaction_prompt: String,
}

/// Builds the actor system: spawns all actors on trouper.
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
    /// Spawn all actors on trouper, build the bus and bridge, and wait for readiness.
    pub async fn build(self) -> (AppCore, Services, jinn_discord::ActivatedDiscord) {
        let ActorSystemBuilderArgs {
            handle,
            llm_service,
            provider_registry,
            api_keys,
            config_storage,
            session_store,
            config,
            app_state_storage,
            paths,
            dump_requests,
            compaction_prompt,
        } = self.args;

        // Create shared State FIRST — injected into multiple actors.
        let state = State::new(AppState::default());

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
        let (initial_session_id, initial_cwd) = {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
            let mut guard = state.write();
            guard.session.set_default_cwd(cwd.clone());
            guard.active_session_mut().set_cwd(cwd.clone());
            (guard.active_session().session_id().clone(), cwd)
        };

        // Create the message fabric: the trouper actor system plus the
        // closure bridge.
        let (bus, trouper_system) = {
            let system =
                trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
            (
                jinn_domain::common::services::bus_service::BusService::new_trouper(system.clone()),
                system,
            )
        };
        let bridge = jinn_domain::common::bridge::Bridge::with_system(&bus, &handle);

        let mut services = Services {
            paths: paths.clone(),
            handle: handle.clone(),
            llm_service: llm_service.clone(),
            provider_registry: provider_registry.clone(),
            api_keys: api_keys.clone(),
            config_storage: config_storage.clone(),
            session_store: session_store.clone(),
            config,
            app_state_storage: app_state_storage.clone(),
            tempdir: None,
            bus,
            bridge: bridge.clone(),
            trouper_system,
            mcp_coordinator: std::sync::Arc::new(std::sync::OnceLock::new()),
            interactive_term: std::sync::Arc::new(std::sync::OnceLock::new()),
            request_dump: jinn_domain::common::request_dump::RequestDumpService::new(dump_requests),
            task_spawns: jinn_tools_msg::TaskSpawnRegistry::default(),
            slices: jinn_slices::Slices::new(),
            key_routes: jinn_slices::route::KeyRoutes::new(),
            viewport: jinn_slices::view::Viewport::new(),
            overlay_views: jinn_slices::OverlayViews::new(),
            project_picker: None,
        };

        let actor_deps = ActorDeps {
            services: services.clone(),
        };

        // ── Dashboard slice ───────────────────────────────────────────
        // Activation mints the cell, spawns the canvas actor FIRST
        // (subscribe is the readiness point, so no lifecycle event from
        // subsequently spawned actors is missed), attaches rows,
        // registers the view + tab. Slice integration is exactly this
        // call.
        #[expect(
            clippy::panic,
            reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
        )]
        if let Err(error) = jinn_dashboard::activate(&mut jinn_dashboard::SliceCtx {
            slices: &services.slices,
            key_routes: &services.key_routes,
            viewport: &mut services.viewport,
            trouper_system: &services.trouper_system,
        }) {
            panic!("dashboard slice activation failed: {error}");
        }

        // ── Discord slice ─────────────────────────────────────────────
        // Activation mints the connection cell, spawns the status
        // actor (the connection authority — after the dashboard so its
        // publications are not missed), resolves the `[discord]`
        // section (fail-fast), creates the gateway kanal channels
        // unconditionally, config-gates the bridge actor, and attaches
        // the `gdc` route row. Slice integration is exactly this call.
        let discord_activated = jinn_discord_activate(&mut services, state.clone()).await;

        // Scope-focus slice: activation mints the interaction cell
        // (focus stack, TUI signals, quit latch). Attaches the handle
        // the FrontendState facade resolves through — before any intent
        // can fire.
        state
            .write()
            .frontend
            .attach_slices(services.slices.clone());
        jinn_scope_focus_activate(&mut services);
        jinn_chat_log_view_activate(&mut services, &state);
        jinn_chat_input_activate(&mut services);
        jinn_cwd_activate(&mut services);
        jinn_skills_activate(&mut services);
        jinn_project_activate(&mut services);
        jinn_preferences_activate(&mut services, state.clone()).await;
        jinn_sidebar_activate(&mut services, state.clone());
        jinn_theme_activate(&mut services);

        // Persona slice: activation scans the persona directories and
        // mints the personas cell; the returned set is published as
        // `PersonasLoaded` below, after the session actor (its sole
        // subscriber) has spawned — the push-once contract.
        let persona_entries = jinn_persona_activate(&mut services);

        // Term slice: registers the terminal tab mirrors cell (written
        // by the coordinator actor, read by the TUI overlay + tools),
        // attaches the keybind rows, the capture key hook, and the
        // overlay geometry + renderer. Composition owns exactly this
        // call. Does NOT mint the shared control registry — wiring owns
        // that set-once static so it can hand the same registry to the
        // coordinator actor spawned below.
        jinn_term::activate(&mut services, &state);

        // Tools slice: activation mints the tools/registry cell (idempotent);
        // the orchestrator actor is spawned below (explicit ordering vs.
        // the MCP coordinator — B1).
        jinn_tools::activate(&mut services, &state);
        // The tool picker is registered by the same slice, after the registry
        // cell it seeds its rows from exists.
        jinn_tools_picker_activate(&mut services);
        jinn_mcp_picker_activate(&mut services);

        // Quake bar slice: activation mints the cell, spawns the actor
        // (submit-log writer), attaches rows, and registers the input
        // hook + overlay geometry. Composition owns exactly this call.
        jinn_status_bar_activate(&mut services);
        jinn_quake_bar_activate(&mut services);

        // ── Session-init slice ────────────────────────────────────────
        // Activation installs the discovery partition set, spawns the
        // supervisor + notifier on trouper, and stages the crossing
        // routes. Must precede the readiness publish at the tail of
        // this function: the supervisor's subscriptions must exist
        // before the first `EnvironmentLoaded` trigger.
        jinn_session_init_activate(&mut services, state.clone());

        // Provider-selection slice: activation mints the provider cell,
        // spawns the provider + discover actors (trouper), and attaches
        // the keybind rows. Must precede the boot trio: boot's
        // provider-init actor receives the cell handle minted here.
        let provider_selection = jinn_provider_selection_activate(&mut services, state.clone());

        // ── Infrastructure actors ──────────────────────────────────────────

        // Boot trio (system-ready, env-init, provider-init) from the boot
        // slice; `boot.ready_rx` blocks the main thread below until
        // `AllActorsSpawned`, and `boot.env_init_path` is the ask target
        // for the startup tail. Receives the provider cell the
        // provider-selection slice minted above.
        let boot = jinn_boot::install_actors(
            &services.trouper_system,
            state.clone(),
            &services,
            provider_selection.provider_cell,
        );

        // Preferences + app-state actors: trouper, installed with the
        // preferences slice's activation wrapper below.
        // ── Domain actors ──────────────────────────────────────────────────

        // Session persistence actor — must spawn before ToolOrchestratorActor so
        // ToolsRegistered subscription is ready when tools register builtins in on_start.
        // Deep mailbox (65_536, Block): the session actor is the single sink for
        // every streaming event (StreamToken, StreamCompleted, ToolBatchCompleted, …)
        // from a provider burst. A small mailbox could fill at the [DONE] peak of a
        // large reasoning turn; the Block policy backpressures publishers rather
        // than dropping, so the terminal `StreamCompleted` can never be lost and
        // the session can never wedge mid-stream. There is no deadlock risk:
        // publishers use fire-and-forget sends (no publisher awaits capacity).
        let token_counter = TiktokenCounter::o200k_base();
        // Token-count slice: activation registers the shared entry-token
        // cache cell and spawns the slice's trouper actors; the returned
        // cache is handed to the session actor (accumulation gate) and
        // the prune workers.
        let entry_token_cache = jinn_token_count_activate(&mut services, state.clone());

        // ── Turn-dispatch slice ─────────────────────────────────────
        // Activation spawns the queue actor (trouper ServiceActor, the
        // turn-queue consumer) and stages its crossing routes. Must
        // precede the env-init tail and the session-actor spawn: the
        // queue actor's subscriptions must exist before any dispatch
        // trigger (an `Idle` phase event or a `DispatchTurn` command)
        // is published.
        jinn_turn_dispatch_activate(&mut services, state.clone());

        // ── Inference slice ─────────────────────────────────────────
        // Activation spawns the inference actor (trouper ServiceActor,
        // the LLM stream driver) and stages its crossing routes. Must
        // precede the env-init tail and the session-actor spawn: the
        // forward relays for `SendToLlmProvider`/`CancelStream` must
        // exist before the first dispatch is published.
        jinn_inference_activate(&mut services);

        // ── Watchdog slice ──────────────────────────────────────────
        // Activation spawns the stall + tool-call watchdog actors
        // (trouper ServiceActors). Must follow the inference activation
        // (it consumes the inference slice's stream contracts) and can
        // precede the env-init tail: the watchdogs only publish.
        jinn_watchdog_activate(&mut services, state.clone());

        // ── Citations slice ─────────────────────────────────────────
        // Activation spawns the citations actor (trouper ServiceActor),
        // which detects citable web sources in tool traffic and flushes
        // `CitationsReceived` once per finished turn. Consumes the tools
        // + inference contracts; only publishes.
        jinn_citations_activate(&mut services);

        // ── Context-assembly slice ─────────────────────────────────────
        // Install the slice's actors on trouper (the stateless assembly
        // service + the context-size actor) and stage the crossing
        // routes; the drain below spawns the relays. The size actor
        // holds `Services` for the assembly ask.
        jinn_context_assembly::install_actors(&services.trouper_system, state.clone(), &services);
        // ── Session store + lifecycle slices ──────────────────────────
        // Three session actors split the former whale: the session-turn actor
        // keeps turn progression and context folds; the store actor owns load,
        // fork, archive, and persist; the lifecycle actor owns setup, teardown,
        // close, and working-directory changes. Each contract has exactly one owner.
        // The session picker's overlay and keys are attached later, where the
        // composition `SliceHost` exists; `activate` only mints the cell the
        // store actor publishes into, which the lifecycle activation threads
        // through to that later pass.
        let session_store_handles = jinn_session_store::activate(&services, state.clone());
        jinn_session_lifecycle_activate(
            &mut services,
            state.clone(),
            &session_store_handles,
            jinn_session_lifecycle_msg::BuiltinRegistry::new(),
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
        );
        let _session = jinn_session_turn::activate(
            &services.trouper_system,
            jinn_session_turn::session_actor::SessionPersistenceActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
                counter: token_counter,
                token_cache: entry_token_cache.clone(),
                image_converter: jinn_llm_support::image_convert::ImageConverterService::system(),
            },
        );

        // Tool orchestrator actor: dispatched batches emit per-call
        // execution events and a final ToolBatchCompleted. Spawned after
        // the session actor (consumes SessionClosed) and before the MCP
        // coordinator (so MCP tool registrations land in a running
        // orchestrator).
        jinn_tools::ToolOrchestratorActor::spawn(
            &services.trouper_system,
            jinn_tools::ToolOrchestratorActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
                services: services.clone(),
                builtin_filter: None,
            },
        );

        // MCP lifecycle actor: activation registers the runtime-only status and
        // stderr projection, then the coordinator becomes its sole writer. The
        // cell must exist before spawn so no status/log event can arrive early.
        let mcp_runtime = jinn_mcp_slice::activate_runtime(&services.slices)
            .expect("MCP runtime cell is registered exactly once");
        let mcp_coordinator_path = jinn_mcp_slice::coordinator::McpCoordinatorActor::spawn(
            &services.trouper_system,
            jinn_mcp_slice::coordinator::McpCoordinatorActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
                runtime: mcp_runtime,
            },
        )
        .await;
        // Expose a handle to the tool layer (restart_mcp_server). Minted from
        // the spawned path by the slice; `OnceLock::set` returns Err if already
        // set — ignore (e.g. test re-seed).
        let _ = services
            .mcp_coordinator
            .set(jinn_mcp_slice::mcp_coordinator_handle(
                services.trouper_system.clone(),
                mcp_coordinator_path,
            ));

        // Interactive-term coordinator: owns PTY sessions across tool calls
        // (the `interactive_term*` tools ask it directly). Spawned with the
        // same lifecycle shape as the MCP coordinator; the per-session
        // control registry goes to the terminal tab (takeover UI) wiring.
        let term_controls = jinn_term_msg::TermControls::default();
        let (term_coordinator_path, _controls) =
            jinn_term::interactive_term_actor::InteractiveTermActor::spawn(
                &services.trouper_system,
                jinn_term::interactive_term_actor::InteractiveTermActorDeps {
                    bus: services.bus.clone(),
                    controls: term_controls.clone(),
                    state: state.clone(),
                    config: services.config.clone(),
                },
            )
            .await;
        let _ = services
            .interactive_term
            .set(std::sync::Arc::new(ActorTermHandle::new(
                services.trouper_system.clone(),
                term_coordinator_path,
            )));
        // Install the shared registry for the IntentHandler's takeover
        // intents (synchronous flips that in-flight tool calls observe
        // mid-drain).
        let _ = jinn_term_msg::TERM_CONTROLS.set(term_controls);

        // Directory lister actor (`@path` file popup).
        let _directory_lister = jinn_domain::feat::file_lister::DirectoryListerActor::spawn(
            &services.trouper_system,
            jinn_domain::feat::file_lister::DirectoryListerActorDeps {
                deps: actor_deps.clone(),
                state: state.clone(),
            },
        );

        // Search index maintenance: message-driven reindex state machine —
        // refreshes its in-memory dirty-session queue when idle and
        // reindexes at most REINDEX_BATCH sessions per heartbeat,
        // publishing the remaining count after every session. The first
        // tick self-kicks after the spawn handshake (B7).
        services
            .bus
            .publish(jinn_domain::common::actor::protocol::event::ActorStarting {
                name: jinn_session_store::search_index_actor::SEARCH_INDEX_ROW_NAME.to_owned(),
                description: Some("SearchIndexActor".to_owned()),
            })
            .await;
        let _search_index = jinn_session_store::search_index_actor::SearchIndexActor::spawn(
            &services.trouper_system,
            jinn_session_store::search_index_actor::SearchIndexActorDeps {
                deps: actor_deps.clone(),
                interval: jinn_session_store::search_index_actor::REINDEX_INTERVAL,
                batch: jinn_session_store::search_index_actor::REINDEX_BATCH,
            },
        );
        services
            .bus
            .publish(jinn_domain::common::actor::protocol::event::ActorStarted {
                name: jinn_session_store::search_index_actor::SEARCH_INDEX_ROW_NAME.to_owned(),
                description: Some("SearchIndexActor".to_owned()),
            })
            .await;

        // Context size actor: trouper, installed with the context-assembly
        // slice's install_actors call above.

        // ── Context-curation slice ──────────────────────────────────
        // Activation spawns the two curation troupers (the prune actor
        // and the compaction actor) and stages their crossing routes.
        // Must precede the session-actor spawn: the actors' subscriptions
        // must exist before any `HistoryAppended` / `TriggerCompaction`
        // publish. Strategy enablement is a construction-time gate — the
        // disabled strategies never reach the actor.
        jinn_context_curation_activate(
            &mut services,
            state.clone(),
            handle.clone(),
            compaction_prompt,
        );

        // Signal system readiness and trigger init chain.
        {
            let bus = services.bus.clone();

            // INVARIANT: the MCP coordinator was spawned above and its
            // subscriptions are live when `spawn` returns, so it has
            // already subscribed to `SessionCreated` and
            // `McpEnablementChanged`. Publishing `EnvironmentLoaded` here
            // triggers the welcome-session seeding, which may publish
            // `McpEnablementChanged` immediately — the subscription must
            // already exist. Do not move this publish ahead of the
            // coordinator spawn.

            // Personas: the persona slice scanned at activation; publish
            // now that every actor (the session actor subscribes to
            // `PersonasLoaded`) is spawned.
            if !persona_entries.entries.is_empty() {
                bus.publish(jinn_persona_msg::PersonasLoaded {
                    personas: persona_entries.entries.clone(),
                    error: None,
                })
                .await;
            }

            // Signal all actors spawned.
            bus.publish(jinn_boot_msg::AllActorsSpawned).await;

            // Ask EnvInitActor for config and publish EnvironmentLoaded to
            // trigger the init chain. The trouper ask has a MANDATORY
            // timeout; this is the one real startup ask path.
            use jinn_boot_msg::GetEnvironmentConfig;
            const ENV_ASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
            match services
                .trouper_system
                .ask(
                    boot.env_init_path.clone(),
                    GetEnvironmentConfig,
                    ENV_ASK_TIMEOUT,
                )
                .await
            {
                Ok(value) => match value
                    .decode::<jinn_boot_msg::EnvironmentConfigReply>()
                    .expect("env reply decodes")
                    .config
                {
                    Some(config) => {
                        bus.publish(jinn_boot_msg::EnvironmentLoaded { config })
                            .await;
                    }
                    None => {
                        tracing::warn!("no provider config found — skipping EnvironmentLoaded");
                    }
                },
                Err(e) => {
                    tracing::error!(err = ?e, "failed to get environment config from EnvInitActor");
                }
            }

            // The boot session's cwd is known here; the session-init
            // supervisor routes from payloads, not shared state. This publish
            // triggers the initial session's discovery through the same
            // payload path as every other session.
            bus.publish(jinn_session_lifecycle_msg::SessionCwdChanged {
                session_id: initial_session_id,
                cwd: initial_cwd,
            })
            .await;
        }

        // Wait for SystemReadyActor to confirm readiness.
        let _ = boot.ready_rx.to_async().recv().await;

        // Build AppCore with shared state and the bridge.
        let core = AppCore {
            state: state.clone(),
            bridge: services.bridge.clone(),
        };

        (core, services, discord_activated)
    }
}

/// Activates the scope-focus slice: its state cell only. No routes,
/// no actors, no view.
fn jinn_scope_focus_activate(services: &mut Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_scope_focus::activate(&mut host);
}

/// Activates the chat-log-view slice: its state cell only. No routes,
/// no actors, no view. Also attaches the registry handle on the session
/// map so every session's view facade resolves the cell.
fn jinn_chat_log_view_activate(services: &mut Services, state: &jinn_domain::State) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_chat_log_view::activate(&mut host);
    state.read().session.attach_slices(services.slices.clone());
    // The layout worker pool and the actor that ends a session load once the
    // chat log has been measured. Spawned here so their subscriptions are
    // live before the first session can be loaded.
    jinn_domain::feat::ui::chat_log::install_layout_actors(&services.trouper_system, state.clone());
}

/// Activates the chat-input slice: its state cell only. No routes, no
/// actors, no view. The session map already carries the attached registry
/// handle, so every session's input facade resolves the cell.
fn jinn_sidebar_activate(services: &mut Services, state: jinn_domain::common::state::State) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_sidebar::activate(&mut host, state);
}

fn jinn_session_lifecycle_activate(
    services: &mut Services,
    state: jinn_domain::common::state::State,
    session_store_handles: &jinn_session_store::SessionStoreHandles,
    builtin_registry: jinn_session_lifecycle_msg::BuiltinRegistry,
    shell: String,
) {
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_session_lifecycle::activate(
        &mut host,
        &services_snapshot,
        state,
        builtin_registry,
        shell,
    );
    jinn_session_lifecycle::activate_picker(&mut host);
    // The session picker: its cell was minted by the store slice's `activate`
    // (the store actor publishes loaded rows into it), so only the overlay,
    // keys, and filter hook are attached here.
    jinn_session_store::activate_session_picker(
        &mut host,
        &session_store_handles.session_picker_cell,
    );
}

fn jinn_cwd_activate(services: &mut Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_cwd::activate(&mut host);
}

/// Activates the skills slice: mints the skill picker's cell and overlay.
///
/// The skill picker is the first picker the slice owns outright — its state
/// lives in a slice cell, its scope is a dynamic `SliceScopeId`, and it
/// renders from that cell rather than through the kernel's picker host.
fn jinn_skills_activate(services: &mut Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_skills::activate(&mut host);
}

/// Activates the project slice's project-add popup over the kernel registries.
fn jinn_project_activate(services: &mut Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    services.project_picker = Some(jinn_project::activate(&mut host).project_picker);
}

async fn jinn_preferences_activate(
    services: &mut Services,
    state: jinn_domain::common::state::State,
) {
    // The two persistence actors spawn here with shared state and services,
    // then subscribe synchronously — this must complete before the env-init
    // tail publishes `EnvironmentLoaded`, which triggers publishes of
    // `UpdateAppState` on first boot.
    let system = services.trouper_system.clone();
    let services_handle = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_preferences::activate(&mut host, &system, services_handle, state);
}

fn jinn_chat_input_activate(services: &mut Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_chat_input::activate(&mut host);
}

/// Activates the theme slice: scans the theme directories once and mints
/// the theme-entries cell. No routes, no actors, no view — the readers
/// are the theme picker's open hook and the app-state actor's resolution.
fn jinn_token_count_activate(
    services: &mut Services,
    state: jinn_domain::common::state::State,
) -> jinn_token_count_msg::HistoryWorkerChatEntryTokenCache {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_token_count::activate(&mut host, state)
}

/// Activates the context-curation slice: builds the config-gated prune
/// strategy list, spawns the two curation troupers (prune + compaction),
/// stages their crossing routes, and drains them.
///
/// Strategy enablement is a construction-time gate — a disabled
/// strategy never reaches the prune actor. The regex strategy additionally skips when its rule list is
/// empty or any rule fails to compile (warn-and-skip, never a launch
/// failure).
fn jinn_context_curation_activate(
    services: &mut Services,
    state: jinn_domain::common::state::State,
    handle: tokio::runtime::Handle,
    compaction_prompt: String,
) {
    use jinn_context_curation::strategies::{
        AnchoredAssistantAutoPruneWorker, BrokenEditAutoPruneWorker,
        ConsecutiveReadsAutoPruneWorker, DoubleEditAutoPruneWorker, EditReadAutoPruneWorker,
        ReadEditAutoPruneWorker, RegexAutoPruneWorker, TodoAutoPruneWorker,
        ToolAgeWindowAutoPruneWorker, TrivialAssistantAutoPruneWorker,
    };
    use jinn_context_curation::worker::HistoryWorker;
    use jinn_token_count_msg::HistoryWorkerChatEntryTokenCache;

    let config = services.config.clone();
    let entry_token_cache = HistoryWorkerChatEntryTokenCache::default();
    let counter = jinn_llm_support::token_estimator::TiktokenCounter::o200k_base();

    // Every strategy is constructed unconditionally and reads its own
    // subsection inside `evaluate`. Gating here instead would make a
    // disabled strategy an ABSENT worker, and then a `reload` could only
    // ever turn a strategy ON -- there would be no worker left to turn
    // off. Reading live makes enablement symmetric in both directions.
    let workers: Vec<Box<dyn HistoryWorker>> = vec![
        Box::new(ReadEditAutoPruneWorker {
            layer: config.clone(),
            config: Default::default(),
        }),
        Box::new(EditReadAutoPruneWorker {
            layer: config.clone(),
            config: Default::default(),
        }),
        Box::new(BrokenEditAutoPruneWorker {
            layer: config.clone(),
            config: Default::default(),
        }),
        Box::new(DoubleEditAutoPruneWorker {
            layer: config.clone(),
            config: Default::default(),
        }),
        Box::new(ConsecutiveReadsAutoPruneWorker {
            layer: config.clone(),
            config: Default::default(),
        }),
        Box::new(ToolAgeWindowAutoPruneWorker {
            layer: config.clone(),
            config: Default::default(),
        }),
        Box::new(TodoAutoPruneWorker {
            layer: config.clone(),
            config: Default::default(),
        }),
        Box::new(TrivialAssistantAutoPruneWorker {
            layer: config.clone(),
            config: Default::default(),
            token_cache: entry_token_cache.clone(),
            counter,
        }),
        Box::new(AnchoredAssistantAutoPruneWorker {
            // The anchor radius and the trivial-assistant floor are both
            // read live; this seed only describes the shape.
            layer: config.clone(),
            config: Default::default(),
            token_cache: entry_token_cache,
            counter,
        }),
        Box::new(RegexAutoPruneWorker::new(config.clone())),
    ];

    let compaction_deps = jinn_context_curation::compaction_actor::CompactionActorDeps {
        services: services.clone(),
        state: state.clone(),
        handle,
        compaction_prompt,
    };

    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_context_curation::activate(&mut host, workers, compaction_deps);
}

fn jinn_persona_activate(services: &mut Services) -> jinn_persona_msg::Personas {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    let personas = jinn_persona::activate(&mut host, &services.paths.personas_dir());
    // The persona picker is registered by the same slice, after discovery: its
    // rows are seeded from the personas cell activate just minted.
    jinn_persona::activate_picker(&mut host);
    personas
}

/// Activates the turn-dispatch slice: spawns the queue actor (trouper
/// ServiceActor) and stages its crossing routes. The queue actor holds a
/// `Services` clone for its bus publishes and the assembly ask.
fn jinn_turn_dispatch_activate(services: &mut Services, state: jinn_domain::common::state::State) {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_turn_dispatch::activate(&mut host, state, services_snapshot);
}

/// Activates the inference slice: spawns the inference actor (trouper
/// ServiceActor) and stages its crossing routes. The actor holds a
/// `Services` clone for its bus publishes and factory resolution.
fn jinn_inference_activate(services: &mut Services) {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_inference::activate(&mut host, services_snapshot);
}

/// Activates the watchdog slice: spawns the stall + tool-call watchdog
/// actors (trouper ServiceActors) on the kernel's trouper system. The
/// actors hold a `Services` clone for their bus publishes; the watchdog
/// knobs are read once from the `State` snapshot at activation.
fn jinn_watchdog_activate(services: &mut Services, state: jinn_domain::State) {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_watchdog::activate(&mut host, &state, services_snapshot);
}

/// Activates the citations slice: spawns the citations actor (trouper
/// ServiceActor) on the kernel's trouper system. The actor holds a
/// `Services` clone for its bus publishes.
fn jinn_citations_activate(services: &mut Services) {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_citations::activate(&mut host, services_snapshot);
}

fn jinn_theme_activate(services: &mut Services) {
    let (themes_dir, system_themes_dir) = {
        (
            services.paths.themes_dir(),
            services.paths.system_themes_dir(),
        )
    };
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_theme_slice::activate(&mut host, &themes_dir, &system_themes_dir);
    // The theme picker is registered by the same slice, after discovery: its
    // rows are seeded from the theme-entries cell activate just minted.
    jinn_theme_slice::activate_picker(&mut host);
}

fn jinn_provider_selection_activate(
    services: &mut Services,
    state: jinn_domain::common::state::State,
) -> jinn_provider_selection::ProviderSelectionHandles {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call
    // (discord-activation pattern).
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    let handles = jinn_provider_selection::activate(&mut host, &services_snapshot, state);
    // The reasoning-effort picker is registered by the same slice, after the
    // actors: it spawns nothing, and its rows are built from the session's
    // own effort when it opens.
    jinn_provider_selection::activate_picker(&mut host);
    // The provider picker mints its own cell here - nothing earlier needs a
    // handle to it - so the activation is self-contained.
    jinn_provider_selection::activate_provider_picker(&mut host, &handles.provider_picker_cell);
    // The endpoint picker continues the same activation: its cell was minted
    // by `activate` (the provider actor needs a handle to publish fetches
    // into), so only the overlay, keys, and filter hook are attached here.
    jinn_provider_selection::activate_endpoint_picker(&mut host, &handles.endpoint_picker_cell);
    handles
}

/// Registers the tool picker: its cell, overlay, keys, and filter hook.
///
/// Separate from `jinn_tools::activate` because that one mints the
/// registry cell the picker's rows are seeded from, so it must run first.
fn jinn_tools_picker_activate(services: &mut Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_tools::activate_picker(&mut host);
}

/// Registers the MCP server inspector: its cell, overlay, keys, and hook.
///
/// Separate from `activate_runtime`, which registers only the status and
/// stderr projection cells the coordinator writes to.
fn jinn_mcp_picker_activate(services: &mut Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_mcp_slice::activate_picker(&mut host);
}

fn jinn_status_bar_activate(services: &mut Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_status_bar::activate(&mut host);
}

fn jinn_quake_bar_activate(services: &mut Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_quake_bar::activate(&mut host);
}

/// Activates the discord slice over the kernel's registries.
///
/// Composition assembles the `SliceHost` borrows plus the services the
/// slice's gateway task needs; the slice returns the parked gateway
/// channels and its validated config for the frontend spawn.
async fn jinn_discord_activate(
    services: &mut Services,
    state: jinn_domain::common::state::State,
) -> jinn_discord::ActivatedDiscord {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_discord::activate(&mut host, &services_snapshot, state)
        .await
        .unwrap_or_else(|error| panic!("discord slice activation failed: {error}"))
}

/// Activates the session-init slice over the kernel's registries.
fn jinn_session_init_activate(services: &mut Services, state: jinn_domain::common::state::State) {
    if let Err(error) = jinn_session_init::activate(services, state) {
        panic!("session-init slice activation failed: {error}");
    }
}

/// The `TermHandle` implementation over the coordinator's trouper path.
///
/// Asks route through the system at the coordinator's static path with the
/// mandatory trouper timeout: the settle wait inside the actor already
/// bounds by `max_wait`, so the outer timeout adds a margin for the round
/// trip (`ask_timeout + SPAWN_ASK_MARGIN` pattern).
#[derive(Debug, Clone)]
pub struct ActorTermHandle {
    system: trouper::system::ActorSystem,
    path: trouper::actor::ActorPath,
}

impl ActorTermHandle {
    pub fn new(system: trouper::system::ActorSystem, path: trouper::actor::ActorPath) -> Self {
        Self { system, path }
    }
}

#[async_trait::async_trait]
impl jinn_term_msg::TermHandle for ActorTermHandle {
    async fn spawn_term(
        &self,
        chat_session_id: jinn_core_types::SessionId,
        command: String,
        cwd: std::path::PathBuf,
        size: (u16, u16),
        max_wait: std::time::Duration,
    ) -> Result<jinn_term_msg::SpawnTermOutcome, jinn_term_msg::TermAskError> {
        let msg = jinn_term_msg::SpawnTerm {
            chat_session_id,
            command,
            cwd,
            size,
            max_wait,
        };
        let reply = self
            .system
            .ask(self.path.clone(), msg, TERM_ASK_TIMEOUT)
            .await;
        decode_reply::<jinn_term_msg::SpawnTermOutcome>(reply)
    }

    async fn send_input(
        &self,
        chat_session_id: jinn_core_types::SessionId,
        text: Option<String>,
        keys: Vec<String>,
        enter: bool,
        max_wait: std::time::Duration,
    ) -> Result<jinn_term_msg::SendTermOutcome, jinn_term_msg::TermAskError> {
        let msg = jinn_term_msg::SendTermInput {
            chat_session_id,
            text,
            keys,
            enter,
            max_wait,
        };
        let reply = self
            .system
            .ask(self.path.clone(), msg, TERM_ASK_TIMEOUT)
            .await;
        decode_reply::<jinn_term_msg::SendTermOutcome>(reply)
    }

    async fn kill_term(
        &self,
        chat_session_id: jinn_core_types::SessionId,
    ) -> Result<jinn_term_msg::KillTermOutcome, jinn_term_msg::TermAskError> {
        let reply = self
            .system
            .ask(
                self.path.clone(),
                jinn_term_msg::KillTerm { chat_session_id },
                TERM_ASK_TIMEOUT,
            )
            .await;
        decode_reply::<jinn_term_msg::KillTermOutcome>(reply)
    }

    fn name(&self) -> &'static str {
        "term-coordinator"
    }
}

/// Outer bound for term asks: the settle wait inside the actor bounds by
/// the message's `max_wait`, so this covers the whole round trip with a
/// margin (the `ask_timeout + SPAWN_ASK_MARGIN` pattern).
const TERM_ASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(75);

/// Decodes a trouper ask reply into the typed outcome, mapping transport
/// failure to the domain-level [`jinn_term_msg::TermAskError`].
fn decode_reply<T: serde::de::DeserializeOwned>(
    reply: Result<trouper::json::Json, error_stack::Report<trouper::context::AskError>>,
) -> Result<T, jinn_term_msg::TermAskError> {
    reply
        .ok()
        .and_then(|value| value.decode::<T>().ok())
        .ok_or(jinn_term_msg::TermAskError)
}
