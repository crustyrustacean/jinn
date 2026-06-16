//! Cucumber `World` for judge-aggregation e2e tests.
//!
//! Builds a complete application via production `actor_wiring::ActorSystemBuilder`
//! with a stateful [`FakeLlmServiceFactory`], then drives real turns through the
//! actor system (no event injection). Used to validate the judge plugin's full
//! loop: `on_turn_end` → `create_session` → child stream → `judgment_*` tool
//! call → `record_verdict` → merged origin message.

use std::sync::Arc;
use std::time::Duration;

use cucumber::World;
use jinn::actor_wiring::{ActorSystemBuilder, ActorSystemBuilderArgs};
use jinn_domain::EnqueueUserMessage;
use jinn_domain::SessionId;
use jinn_domain::ToolCall;
use jinn_domain::common::bridge::Bridge;
use jinn_domain::feat::plugin_dispatch::protocol::command::AttachPlugin;
use jinn_domain::feat::session::model_selection::ModelSelection;
use jinn_domain::{
    ApiKeys, ApiKeysService, AppStateStorageService, ConfigStorageService, FakeLlmServiceFactory,
    InMemoryAppStateStorage, InMemoryConfigStorage, InMemoryUserPreferencesStorage,
    LlmServiceFactoryService, ProviderRegistry, ProviderRegistryService, ProvidersConfig,
    ScriptedResponse, SessionStoreService, SqliteSessionStore, UserPreferencesStorageService,
};
use jinn_domain::{ChatEntry, ChatEntryKind};

/// A verdict kind queued via the scripted LLM factory.
#[derive(Debug, Clone)]
enum Verdict {
    Pass,
    Fail { message: String },
}

/// Cucumber world for judge-aggregation scenarios.
///
/// Owns a fully-wired [`AppCore`] (all 16 actors) with fake services. Each
/// scenario constructs a fresh world via [`Self::new_judge_world`].
#[derive(World)]
#[world(init = Self::new_judge_world)]
pub struct JudgeWorld {
    /// The application core: shared state + command bridge.
    core: jinn_domain::AppCore,
    /// The tokio runtime that owns the actor system. Kept so `Drop` can shut
    /// it down between scenarios (kameo's actor registry is process-global,
    /// so a leaked runtime deadlocks the next build's `register("env-init")`).
    runtime: Option<tokio::runtime::Runtime>,
    /// The root supervisor — stopped in `Drop` to tear down every actor
    /// (which unregisters `env-init` from the global registry).
    root_supervisor: Option<jinn_domain::common::root_supervisor::RootSupervisorRef>,
    /// The stateful fake LLM factory — steps push scripted responses onto it.
    fake_factory: Arc<FakeLlmServiceFactory>,
    /// Temp directory holding all test filesystem paths. Cleaned up on drop.
    #[allow(dead_code)]
    temp_dir: tempfile::TempDir,
}

impl std::fmt::Debug for JudgeWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JudgeWorld").finish_non_exhaustive()
    }
}

impl Drop for JudgeWorld {
    fn drop(&mut self) {
        // Stop the actor system (unregisters "env-init" from kameo's global
        // registry), then drop the runtime to free its worker threads.
        //
        // Must run on a dedicated std thread: `block_on` panics if called from
        // within an existing tokio runtime context (cucumber's #[tokio::main]).
        // The child process exits immediately after Drop, so the OS reclaims
        // anything this graceful shutdown might miss.
        if let (Some(root), Some(rt)) = (self.root_supervisor.take(), self.runtime.take()) {
            let join = std::thread::spawn(move || {
                rt.handle().block_on(async {
                    let _ = root.stop_gracefully().await;
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        root.wait_for_shutdown(),
                    )
                    .await;
                });
            });
            let _ = join.join();
        }
    }
}

impl JudgeWorld {
    /// Creates a new world with the full production actor wiring + fake services.
    fn new_judge_world() -> Self {
        let temp_dir = tempfile::TempDir::new().expect("test temp dir");
        let temp_path = temp_dir.path().to_path_buf();

        // Make the real judge plugin discoverable: the plugin loader scans
        // <config>/jinn/plugins/attachable/. Copy the repo's bundled judge
        // plugin (resolved via the workspace root) into the temp config tree
        // so attach("judge") resolves a real init.lua in the test process.
        {
            let manifest_dir = option_env!("CARGO_MANIFEST_DIR").unwrap_or(".").to_string();
            let repo_root = std::path::Path::new(&manifest_dir)
                .ancestors()
                .nth(2)
                .expect("workspace root");
            let src = repo_root.join("res/plugins/attachable/judge/init.lua");
            let dst_dir = temp_path.join("config/jinn/plugins/attachable/judge");
            std::fs::create_dir_all(&dst_dir).unwrap_or_else(|e| panic!("mkdir {dst_dir:?}: {e}"));
            std::fs::copy(&src, dst_dir.join("init.lua"))
                .unwrap_or_else(|e| panic!("copy {src:?} -> {dst_dir:?}: {e}"));
        }

        let temp_path = temp_dir.path().to_path_buf();
        let fake_factory = Arc::new(FakeLlmServiceFactory::new(vec![]));

        // Build on a dedicated runtime (the builder's `build()` is async and
        // must run inside a tokio context).
        let (core, runtime, root_supervisor) = {
            let (tx, rx) = std::sync::mpsc::channel();
            let fake_factory = fake_factory.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("test runtime");
                let handle = rt.handle().clone();

                let config_storage =
                    ConfigStorageService::new(Arc::new(InMemoryConfigStorage::new()));
                let resolved_api_keys = ApiKeysService::new(ApiKeys::new());
                let empty_config = ProvidersConfig {
                    providers: vec![],
                    aliases: vec![],
                    default_provider: None,
                };
                let provider_registry = ProviderRegistryService::new(
                    ProviderRegistry::from_config(empty_config).expect("empty config is valid"),
                );
                let llm_service = LlmServiceFactoryService::new(fake_factory);
                let user_preferences_storage = {
                    let svc = UserPreferencesStorageService::new(Arc::new(
                        InMemoryUserPreferencesStorage::new(),
                    ));
                    svc.reload().expect("test prefs storage initial reload");
                    svc
                };
                let paths = jinn_domain::AppPaths::new_in(&temp_path);
                let session_store = SessionStoreService::new(Arc::new(
                    rt.handle()
                        .block_on(SqliteSessionStore::new_in(&paths.sessions_dir()))
                        .expect("store"),
                ));
                let app_state_storage =
                    AppStateStorageService::new(Arc::new(InMemoryAppStateStorage::new()));
                app_state_storage
                    .reload()
                    .expect("test state storage initial reload");

                let (core, services, _sync_plugins) = rt.handle().block_on(async {
                    ActorSystemBuilder::new(ActorSystemBuilderArgs {
                        handle: handle.clone(),
                        llm_service,
                        provider_registry,
                        api_keys: resolved_api_keys,
                        config_storage,
                        session_store,
                        user_preferences_storage,
                        app_state_storage,
                        paths,
                    })
                    .build()
                    .await
                });
                let root_supervisor = services.root_supervisor.clone();
                tx.send((core, rt, root_supervisor))
                    .expect("send setup results");
            });

            rx.recv().expect("receive setup results")
        };

        Self {
            core,
            runtime: Some(runtime),
            root_supervisor: Some(root_supervisor),
            fake_factory,
            temp_dir,
        }
    }

    /// Returns the active session id.
    fn active_session_id(&self) -> SessionId {
        self.core.state.read().session.active_session_id().clone()
    }

    /// Publishes a bus message to the actor system.
    fn publish<M>(&self, msg: M)
    where
        M: jinn_domain::common::bus::BusMessage,
    {
        let _ = self.core.bridge.send(Bridge::publish_closure(msg));
    }

    /// Queues a scripted origin-turn response (text tokens, no tool calls).
    fn queue_origin_turn(&self, text: &str) {
        self.fake_factory.push_scripted_response(ScriptedResponse {
            tokens: vec![text.to_owned()],
            tool_calls: vec![],
        });
    }

    /// Queues a scripted judge verdict tool call.
    fn queue_verdict(&self, verdict: &Verdict) {
        let tool_call = match verdict {
            Verdict::Pass => ToolCall {
                id: "tc-pass".to_owned(),
                name: "judgment_passed".to_owned(),
                arguments: "{}".to_owned(),
            },
            Verdict::Fail { message } => ToolCall {
                id: "tc-fail".to_owned(),
                name: "judgment_failed".to_owned(),
                arguments: format!("{{\"message\":{}}}", serde_json::json!(message)),
            },
        };
        self.fake_factory.push_scripted_response(ScriptedResponse {
            tokens: vec![],
            tool_calls: vec![tool_call],
        });
    }

    /// Polls shared state until `predicate` holds, returning whether it was
    /// observed before the deadline. Returns the observed state for an
    /// assertion (avoids re-locking after the poll, which races with
    /// transient-entry clearing).
    async fn wait_until<F, T>(&self, predicate: F) -> Option<T>
    where
        F: Fn(&jinn_domain::AppState) -> Option<T>,
    {
        let state = self.core.state.clone();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(v) = predicate(&state.read()) {
                return Some(v);
            }
            if tokio::time::Instant::now() > deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

// ─── Step definitions ───────────────────────────────────────────────────

#[cucumber::given(expr = "a fresh app")]
fn given_fresh_app(_world: &mut JudgeWorld) {
    // The world's constructor already built a fresh app. This step exists for
    // Gherkin readability; it is a no-op.
}

#[cucumber::given(expr = "the active provider is set")]
fn given_active_provider_set(world: &mut JudgeWorld) {
    world
        .core
        .state
        .write()
        .session
        .active_session_mut()
        .set_model(ModelSelection::default());
}

#[cucumber::given(expr = "the app attaches the plugin {string}")]
async fn given_attach_plugin(world: &mut JudgeWorld, plugin_name: String) {
    let session_id = world.active_session_id();
    world.publish(AttachPlugin {
        session_id: session_id.clone(),
        plugin_name,
    });
    // Wait until the attachment lands on the session's attached_plugins vec
    // (proves the dispatch actor processed the attach).
    let attached = world
        .wait_until(|s| {
            s.session
                .get(&session_id)
                .is_some_and(|sess| sess.core.attached_plugins.iter().any(|p| p.name == "judge"))
                .then_some(())
        })
        .await;
    assert!(attached.is_some(), "judge plugin never attached");
    // The registry load (recreate_session_registry) + on_attach fire
    // happen async on the plugin thread and are not observable from app
    // state. Wait a bounded, generous amount so the origin's Idle
    // transition finds the registry populated.
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[cucumber::given(expr = "the app queues a scripted origin turn with text {string}")]
fn given_queue_origin_turn(world: &mut JudgeWorld, text: String) {
    world.queue_origin_turn(&text);
}

#[cucumber::given(expr = "the app queues a scripted judgment {string} with message {string}")]
fn given_queue_verdict(world: &mut JudgeWorld, verdict: String, message: String) {
    let v = match verdict.as_str() {
        "judgment_passed" => Verdict::Pass,
        "judgment_failed" => Verdict::Fail { message },
        other => panic!("unknown verdict kind: {other}"),
    };
    world.queue_verdict(&v);
}

#[cucumber::when(expr = "the app submits an EnqueueUserMessage with text {string}")]
async fn when_enqueue_user_message(world: &mut JudgeWorld, text: String) {
    let session_id = world.active_session_id();
    world.publish(EnqueueUserMessage {
        session_id,
        entry: ChatEntry::user(text),
    });
    // Wait for the origin's assistant entry to appear, proving the full
    // stream completed (fake factory pops the scripted origin-turn response).
    world
        .wait_until(|s| {
            s.session
                .active_session()
                .history()
                .iter()
                .any(|e| matches!(&e.kind, ChatEntryKind::Assistant(_)))
                .then_some(())
        })
        .await;
}

#[cucumber::then(expr = "the origin session final entry is a transient {string}")]
async fn then_final_entry_transient(world: &mut JudgeWorld, expected: String) {
    let expected_for_predicate = expected.clone();
    // Bind the result INSIDE wait_until to avoid the race where a
    // transient entry is observed by the poll but cleared before the
    // re-check (e.g. the aggregator disables plugins and a follow-on
    // action clears transients).
    let held = world
        .wait_until(|s| {
            s.session
                .active_session()
                .history()
                .iter()
                .any(|e| matches!(&e.kind, ChatEntryKind::Transient(t) if t.contains(&expected_for_predicate)))
                .then_some(())
        })
        .await;
    if held.is_none() {
        let dump = world
            .core
            .state
            .read()
            .session
            .active_session()
            .history()
            .iter()
            .map(|e| format!("{:?}", e.kind))
            .collect::<Vec<_>>()
            .join("\n  ");
        let phase = world.core.state.read().session.active_session().phase();
        let attached = world
            .core
            .state
            .read()
            .session
            .active_session()
            .core
            .attached_plugins
            .iter()
            .map(|p| format!("{}(enabled={})", p.name, p.enabled))
            .collect::<Vec<_>>()
            .join(", ");
        panic!(
            "no transient entry containing {expected:?}\nPhase: {phase:?}\nAttached: {attached}\nHistory:\n  {dump}"
        );
    }
}

#[cucumber::then(
    expr = "the origin session final entry is a failed user message containing {string}"
)]
async fn then_final_entry_fail_message(world: &mut JudgeWorld, expected: String) {
    let expected_for_predicate = expected.clone();
    // Bind the result INSIDE wait_until (same race rationale as the
    // transient step — see above).
    let held = world
        .wait_until(|s| {
            s.session
                .active_session()
                .history()
                .iter()
                .any(|e| {
                    matches!(
                        &e.kind,
                        ChatEntryKind::User { display, .. } if display.contains(&expected_for_predicate)
                    )
                })
                .then_some(())
        })
        .await;
    if held.is_none() {
        let dump = world
            .core
            .state
            .read()
            .session
            .active_session()
            .history()
            .iter()
            .map(|e| format!("{:?}", e.kind))
            .collect::<Vec<_>>()
            .join("\n  ");
        let phase = world.core.state.read().session.active_session().phase();
        panic!(
            "no failed user message containing {expected:?}\nPhase: {phase:?}\nHistory:\n  {dump}"
        );
    }
}
