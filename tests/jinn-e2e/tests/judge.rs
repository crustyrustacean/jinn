//! Cucumber `World` for judge-aggregation e2e tests.
//!
//! Builds a full [`TuiApp`] (rendering disabled) via the shared harness with a
//! stateful [`FakeLlmServiceFactory`], then drives real turns through the
//! actor system (no event injection). Validates the judge plugin's full loop:
//! `on_turn_end` → `create_session` → child stream → `judgment_*` tool call →
//! `record_verdict` → merged origin message.

use std::sync::Arc;
use std::time::Duration;

use cucumber::World;
use jinn_domain::EnqueueUserMessage;
use jinn_domain::SessionId;
use jinn_domain::ToolCall;
use jinn_domain::common::bridge::Bridge;
use jinn_domain::feat::plugin_dispatch::protocol::command::AttachPlugin;
use jinn_domain::feat::session::model_selection::ModelSelection;
use jinn_domain::{ChatEntry, ChatEntryKind, FakeLlmServiceFactory, ScriptedResponse};
use jinn_tui::TuiApp;

use crate::harness::{build_tuiapp_in_temp, copy_plugin_to_temp};

/// A verdict kind queued via the scripted LLM factory.
#[derive(Debug, Clone)]
enum Verdict {
    Pass,
    Fail { message: String },
}

/// Cucumber world for judge-aggregation scenarios.
///
/// Holds only what isn't reachable through [`TuiApp`]: the `tuiapp` itself, the
/// typed fake LLM factory (its queueing API is erased behind `Arc<dyn>` inside
/// the app), and the temp dir. Everything else (`services`, `bridge`,
/// `root_supervisor`, `plugins`) is read off `tuiapp` on demand.
#[derive(World)]
#[world(init = Self::new_judge_world)]
pub struct JudgeWorld {
    /// The full app (rendering disabled). Drives the real keystroke/command
    /// paths — same surface users hit.
    tuiapp: TuiApp,
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

impl JudgeWorld {
    /// Creates a new world via the shared harness: temp dir, judge plugin
    /// copied into the config tree, then a real actor system wrapped in a
    /// `TuiApp`.
    async fn new_judge_world() -> Self {
        let temp_dir = tempfile::TempDir::new().expect("test temp dir");
        copy_plugin_to_temp(temp_dir.path(), "judge");
        let fake_factory = Arc::new(FakeLlmServiceFactory::new(vec![]));
        let tuiapp = build_tuiapp_in_temp(temp_dir.path(), fake_factory.clone()).await;
        Self {
            tuiapp,
            fake_factory,
            temp_dir,
        }
    }

    /// Returns the active session id.
    fn active_session_id(&self) -> SessionId {
        self.tuiapp
            .core
            .state
            .read()
            .session
            .active_session_id()
            .clone()
    }

    /// Publishes a bus message to the actor system.
    fn publish<M>(&self, msg: M)
    where
        M: jinn_domain::common::bus::BusMessage,
    {
        let _ = self.tuiapp.core.bridge.send(Bridge::publish_closure(msg));
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
        let state = self.tuiapp.core.state.clone();
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
        .tuiapp
        .core
        .state
        .write()
        .session
        .active_session_mut()
        .set_model(ModelSelection::Single("test".to_owned()));
}

#[cucumber::given(expr = "the app attaches the plugin {string}")]
async fn given_attach_plugin(world: &mut JudgeWorld, plugin_name: String) {
    world.publish(AttachPlugin {
        session_id: world.active_session_id(),
        plugin_name,
    });
    // Give the dispatch actor time to load the plugin and fire on_attach.
    tokio::time::sleep(Duration::from_millis(50)).await;
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
            .tuiapp
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
        let phase = world
            .tuiapp
            .core
            .state
            .read()
            .session
            .active_session()
            .phase();
        let attached = world
            .tuiapp
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
            .tuiapp
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
        let phase = world
            .tuiapp
            .core
            .state
            .read()
            .session
            .active_session()
            .phase();
        panic!(
            "no failed user message containing {expected:?}\nPhase: {phase:?}\nHistory:\n  {dump}"
        );
    }
}
