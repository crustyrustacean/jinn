//! Cucumber `World` for gap-analysis-plugin e2e tests.
//!
//! Builds a full [`TuiApp`] (rendering disabled) via the shared harness with a
//! stateful [`FakeLlmServiceFactory`], then drives real turns through the
//! actor system (no event injection). Validates the gap-analysis attachable
//! plugin's full loop: task list driven to completion → session goes Idle →
//! `on_turn_end` fires → plugin enqueues `#gap-analysis` → the user entry is
//! expanded against the session's prompt store into origin history.
//!
//! Step definitions live below the `World` impl. They are filled in alongside
//! the `.feature` scenarios (Phase 3); the struct + constructor land here so
//! the runner's [`WorldKind::GapAnalysis`] dispatch arm compiles.

use std::sync::Arc;
use std::time::Duration;

use cucumber::World;
use jinn_domain::PhaseKind;
use jinn_domain::SessionId;
use jinn_domain::ToolCall;
use jinn_domain::common::bridge::Bridge;
use jinn_domain::feat::plugin_dispatch::protocol::command::AttachPlugin;
use jinn_domain::feat::session::model_selection::ModelSelection;
use jinn_domain::{
    ChatEntry, ChatEntryKind, EnqueueUserMessage, FakeLlmServiceFactory, ScriptedResponse,
};
use jinn_tui::TuiApp;

use crate::harness::{
    build_tuiapp_in_temp, copy_plugin_to_temp, prompt_template_ready_predicate,
    seed_prompt_template,
};

/// Cucumber world for gap-analysis-plugin scenarios.
///
/// Mirrors [`JudgeWorld`](crate::judge::JudgeWorld)'s shape: holds only what
/// isn't reachable through [`TuiApp`] — the `tuiapp` itself, the typed fake
/// LLM factory (its queueing API is erased behind `Arc<dyn>` inside the app),
/// and the temp dir. Everything else is read off `tuiapp` on demand.
#[derive(World)]
#[world(init = Self::new_gap_analysis_world)]
pub struct GapAnalysisWorld {
    /// The full app (rendering disabled). Drives the real command/keystroke
    /// paths — same surface users hit.
    tuiapp: TuiApp,
    /// The stateful fake LLM factory — steps push scripted responses onto it.
    fake_factory: Arc<FakeLlmServiceFactory>,
    /// Temp directory holding all test filesystem paths. Cleaned up on drop.
    #[allow(dead_code)]
    temp_dir: tempfile::TempDir,
}

impl std::fmt::Debug for GapAnalysisWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GapAnalysisWorld").finish_non_exhaustive()
    }
}

impl GapAnalysisWorld {
    /// Creates a new world via the shared harness: temp dir, gap-analysis
    /// plugin copied into the config tree, then a real actor system wrapped
    /// in a `TuiApp`.
    async fn new_gap_analysis_world() -> Self {
        let temp_dir = tempfile::TempDir::new().expect("test temp dir");
        copy_plugin_to_temp(temp_dir.path(), "gap-analysis");
        // Seed the prompt template BEFORE build so prompt_scan_actor populates
        // the store at startup and `#gap-analysis` resolves on push_entry.
        seed_prompt_template(temp_dir.path(), "gap-analysis", "Run the gap analysis.");
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

    /// Polls until the session is in the Idle phase and has remained there
    /// continuously for a grace window, then returns.
    ///
    /// The grace window absorbs the plugin's async `on_turn_end` cascade: when a
    /// turn ends the session briefly hits Idle, the plugin fires and enqueues a
    /// contingency turn (breaking Idle), and only returns to sustained Idle once
    /// that turn completes. Without sustained-Idle, the next step's FIFO push
    /// races the in-flight contingency turn.
    async fn wait_until_settled(&self) {
        let state = self.tuiapp.core.state.clone();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let grace = Duration::from_millis(300);
        loop {
            // Wait for the first Idle observation.
            while state.read().session.active_session().phase() != PhaseKind::Idle {
                if tokio::time::Instant::now() > deadline {
                    let phase = state.read().session.active_session().phase();
                    panic!("session never reached Idle: phase={phase:?}");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            // Now require Idle to persist for the full grace window. Any phase
            // change (e.g. the plugin's contingency turn starting) resets the
            // window.
            let sustained_until = tokio::time::Instant::now() + grace;
            loop {
                if tokio::time::Instant::now() >= sustained_until {
                    return; // sustained Idle — settled
                }
                if state.read().session.active_session().phase() != PhaseKind::Idle {
                    break; // phase changed mid-grace; restart outer loop
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("session never sustained Idle for {grace:?}");
            }
        }
    }

    /// Queues a scripted LLM turn whose only action is a `todo_set_list` call
    /// that creates a single empty phase — i.e. immediately complete.
    ///
    /// An empty phase has no pending work (`has_pending_work() == false`), so the
    /// whole list is `is_complete`. This sidesteps the random-task-id constraint:
    /// completion via `todo_complete_task` would require parsing the rendered list
    /// for IDs (task ids are randomized via `generate_id_chars`).
    fn queue_complete_task_list(&self) {
        let arguments = serde_json::json!({
            "phases": [{ "description": "Done" }]
        })
        .to_string();
        self.fake_factory.push_scripted_response(ScriptedResponse {
            tokens: vec![],
            tool_calls: vec![ToolCall {
                id: "tc-set-complete".to_owned(),
                name: "todo_set_list".to_owned(),
                arguments,
            }],
        });
    }

    /// Queues a scripted LLM turn whose `todo_set_list` creates one phase with
    /// one pending task — i.e. not complete.
    fn queue_pending_task_list(&self) {
        let arguments = serde_json::json!({
            "phases": [{ "description": "Work", "tasks": ["do thing"] }]
        })
        .to_string();
        self.fake_factory.push_scripted_response(ScriptedResponse {
            tokens: vec![],
            tool_calls: vec![ToolCall {
                id: "tc-set-pending".to_owned(),
                name: "todo_set_list".to_owned(),
                arguments,
            }],
        });
    }

    /// Queues a scripted text-only origin turn (no tool calls). Used to drive a
    /// subsequent Idle transition after a task-list tool call.
    fn queue_text_turn(&self, text: &str) {
        self.fake_factory.push_scripted_response(ScriptedResponse {
            tokens: vec![text.to_owned()],
            tool_calls: vec![],
        });
    }
}

/// Sets the active session's model to `test/test` (the fake-provider id wired
/// by the shared harness).
///
/// A free function rather than a step so both `Given` steps and helpers can
/// call it; the harness's `FakeLlmServiceFactory` only resolves for that id.
pub(crate) fn set_active_provider(world: &GapAnalysisWorld) {
    world
        .tuiapp
        .core
        .state
        .write_test()
        .session
        .active_session_mut()
        .set_model(ModelSelection::Single("test/test".to_owned()));
}

// ─── Step definitions ───────────────────────────────────────────────────

/// The expanded body of the seeded `#gap-analysis` template. Kept in sync
/// with the ctor's `seed_prompt_template` call.
const GAP_ANALYSIS_BODY: &str = "Run the gap analysis.";

#[cucumber::given(expr = "a fresh app")]
fn given_fresh_app(_world: &mut GapAnalysisWorld) {
    // The world's constructor already built a fresh app. No-op.
}

#[cucumber::given(expr = "the active provider is set")]
fn given_active_provider_set(world: &mut GapAnalysisWorld) {
    set_active_provider(world);
}

#[cucumber::given(expr = "the app has a prompt template {string} with body {string}")]
async fn given_prompt_template(world: &mut GapAnalysisWorld, name: String, _body: String) {
    // The template is seeded in the world ctor (before build) so
    // prompt_scan_actor populates the store at startup. This step waits for
    // that scan to complete so subsequent turns can expand `#name`.
    let held = world
        .wait_until(prompt_template_ready_predicate(&name))
        .await;
    if held.is_none() {
        panic!("prompt template {name:?} never discovered within deadline");
    }
}

#[cucumber::given(expr = "the app attaches the plugin {string}")]
async fn given_attach_plugin(world: &mut GapAnalysisWorld, plugin_name: String) {
    world.publish(AttachPlugin {
        session_id: world.active_session_id(),
        plugin_name,
    });
    // on_attach is dispatched via tokio::spawn inside handle_attach and is not
    // awaited, so no AppState observable reflects its completion. A short
    // bounded sleep lets the detached on_attach (Lua state init) settle before
    // subsequent steps drive a turn. See plan.md divergence note.
    tokio::time::sleep(Duration::from_millis(50)).await;
}

/// Queue the responses for one complete-and-end sequence: the task-list
/// tool call (turn 1), a text turn to drive the tool-loop continuation to
/// Idle (turn 2), and a final text response for the plugin-enqueued turn.
fn queue_complete_and_end(world: &GapAnalysisWorld) {
    world.queue_complete_task_list();
    world.queue_text_turn("done");
    world.queue_text_turn("ok");
}

fn queue_pending_and_end(world: &GapAnalysisWorld) {
    world.queue_pending_task_list();
    world.queue_text_turn("done");
    // No third response: a pending list never fires the plugin, so no
    // contingency turn is triggered. (Compare `queue_complete_and_end`, which
    // adds a third response for the plugin's enqueued turn.)
}

fn queue_idle_turn(world: &GapAnalysisWorld) {
    world.queue_text_turn("ok");
}

/// Triggers an origin turn by enqueueing a user message and waits until the
// session fully settles: the scripted-LLM FIFO is drained, the phase is Idle,
// and a grace window elapses to absorb the plugin's async on_turn_end cascade
// (which enqueues a contingency turn that itself pops the FIFO).
///
// This ordering is critical: returning early (e.g. on the first assistant
// entry) lets the next step push onto a FIFO the in-flight plugin turn will
// later consume, corrupting the scripted sequence.
async fn run_turn(world: &GapAnalysisWorld, text: &str) {
    let session_id = world.active_session_id();
    world.publish(EnqueueUserMessage {
        session_id,
        entry: ChatEntry::user(text),
    });
    world.wait_until_settled().await;
}

#[cucumber::when(expr = "the app completes the task list then ends the turn")]
async fn when_complete_task_list(world: &mut GapAnalysisWorld) {
    queue_complete_and_end(world);
    run_turn(world, "go").await;
}

#[cucumber::when(expr = "the app sets a pending task list then ends the turn")]
async fn when_pending_task_list(world: &mut GapAnalysisWorld) {
    queue_pending_and_end(world);
    run_turn(world, "go").await;
}

#[cucumber::when(expr = "the app ends another turn without changing the list")]
async fn when_another_turn(world: &mut GapAnalysisWorld) {
    queue_idle_turn(world);
    run_turn(world, "again").await;
}

/// Collects the `expanded` text of every `User` entry containing the
/// template body. Bound inside the poll to avoid re-lock races.
fn expanded_gap_entries(s: &jinn_domain::AppState) -> Vec<String> {
    s.session
        .active_session()
        .history()
        .iter()
        .filter_map(|e| match &e.kind {
            ChatEntryKind::User { expanded, .. } if expanded.contains(GAP_ANALYSIS_BODY) => {
                Some(expanded.clone())
            }
            _ => None,
        })
        .collect()
}

#[cucumber::then(expr = "the origin session history gains an expanded {string} entry")]
async fn then_gains_expanded_entry(world: &mut GapAnalysisWorld, _name: String) {
    let held = world
        .wait_until(|s| (!expanded_gap_entries(s).is_empty()).then_some(()))
        .await;
    if held.is_none() {
        let dump = history_dump(world);
        panic!("no expanded `#gap-analysis` entry in history\n{dump}");
    }
}

#[cucumber::then(
    expr = "the origin session history has no user entry containing the literal {string} token"
)]
async fn then_no_literal_token(world: &mut GapAnalysisWorld, token: String) {
    let offending = world
        .tuiapp
        .core
        .state
        .read()
        .session
        .active_session()
        .history()
        .iter()
        .filter_map(|e| match &e.kind {
            ChatEntryKind::User { expanded, .. } if expanded.contains(&token) => {
                Some(format!("expanded={expanded:?}"))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if !offending.is_empty() {
        let dump = history_dump(world);
        panic!("found literal {token:?} in a user entry: {offending:?}\n{dump}");
    }
}

#[cucumber::then(expr = "the origin session history has no expanded {string} entry")]
async fn then_no_expanded_entry(world: &mut GapAnalysisWorld, _name: String) {
    let entries = expanded_gap_entries(&world.tuiapp.core.state.read());
    if !entries.is_empty() {
        let dump = history_dump(world);
        panic!("unexpected expanded `#gap-analysis` entries: {entries:?}\n{dump}");
    }
}

#[cucumber::then(expr = "the origin session history has exactly one expanded {string} entry")]
async fn then_exactly_one(world: &mut GapAnalysisWorld, _name: String) {
    let held = world
        .wait_until(|s| (expanded_gap_entries(s).len() == 1).then_some(()))
        .await;
    if held.is_none() {
        let dump = history_dump(world);
        panic!("expected exactly one expanded `#gap-analysis` entry\n{dump}");
    }
}

/// Renders the active session's history + phase as a debug string for
/// assertion-failure diagnostics.
fn history_dump(world: &GapAnalysisWorld) -> String {
    let state = world.tuiapp.core.state.read();
    let session = state.session.active_session();
    let dump = session
        .history()
        .iter()
        .map(|e| format!("  {:?}", e.kind))
        .collect::<Vec<_>>()
        .join("\n");
    let phase = session.phase();
    format!("Phase: {phase:?}\nHistory:\n{dump}")
}
//
// Filled in alongside the `.feature` scenarios (Phase 3). Declared here so the
// module's shape is visible; bodies are added when the feature is written.
