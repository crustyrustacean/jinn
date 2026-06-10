//! Bench actor - orchestrates bench execution and records results.
//!
//! When given a [`BenchPlan`], the actor drives the full bench pipeline:
//! creates sessions with bench lifecycle names, enqueues messages, waits for
//! completion, runs verification, writes CSV rows, and advances to the next
//! task/model pair.
//!
//! Subscribes to [`SessionSetupCompleted`], [`StreamCompleted`], and
//! [`SessionPhaseChanged`] events.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::csv::{BenchCsvWriter, BenchResult};
use crate::orchestrator::BenchPlan;
use crate::task::{BenchTask, VerificationReport};
use crate::tasks;
use jinn_domain::feat::chat_input::protocol::command::{EnqueueUserMessage, PushChatEntry};
use jinn_domain::feat::provider::protocol::command::CancelStream;
use jinn_domain::feat::provider::protocol::event::StreamCompleted;
use jinn_domain::feat::session::chat_session::ChatSessionState;
use jinn_domain::feat::session::phase_machine::PhaseKind;
use jinn_domain::feat::session::profile::SessionProfile;
use jinn_domain::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use jinn_domain::feat::session::session_actor::setup_running_msg;
use jinn_domain::feat::session::token_stats::TokenStats;
use jinn_domain::feat::session_lifecycle::builtin::{BuiltinId, LifecycleCommand};
use jinn_domain::feat::session_lifecycle::protocol::command::RunSessionSetup;
use jinn_domain::feat::session_lifecycle::protocol::event::SessionSetupCompleted;
use jinn_domain::protocol::{ChatEntry, Command, Event, SessionId};
use jinn_domain::{Actor, ActorContext, ActorEnvelope, NoDirectMsg, State};

/// A tracked bench session.
struct BenchSession {
    /// The bench task name (matches the lifecycle name).
    task_name: String,
    /// When this session was first tracked (after setup completed).
    start_time: Instant,
    /// Deadline for timeout - `start_time + task.timeout`.
    deadline: Instant,
    /// Verification function from the task definition.
    verify: fn(&std::path::Path) -> VerificationReport,
    /// How many messages still need to be sent for this task.
    messages_remaining: usize,
    /// Index of the next message to send in the task's message list.
    next_message_index: usize,
}

/// The bench actor.
///
/// When given a plan, orchestrates bench execution by creating sessions,
/// enqueuing messages, and recording results. Without a plan, acts as a
/// passive observer (backward compatible with non-bench mode).
pub struct BenchActor {
    /// Shared application state.
    state: State,
    /// Tracked bench sessions - keyed by session ID.
    pending: HashMap<SessionId, BenchSession>,
    /// CSV writer for results.
    csv_writer: Option<BenchCsvWriter>,
    /// Lookup from task name → BenchTask definition.
    task_lookup: HashMap<String, BenchTask>,
    /// The execution plan (models × tasks).
    plan: Option<BenchPlan>,
    /// Index into `plan.pairs` for the next pair to start.
    current_pair_index: usize,
}

/// Dependencies for [`BenchActor`].
pub struct BenchActorDeps {
    /// Shared application state.
    pub state: State,
    /// Path to write CSV output. If `None`, results are logged but not written.
    pub csv_path: Option<PathBuf>,
    /// The execution plan. If `None`, the actor is passive.
    pub plan: Option<BenchPlan>,
}

impl Actor for BenchActor {
    type Message = NoDirectMsg;
    type Deps = BenchActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Orchestrates bench sessions and writes CSV results");
        ctx.subscribe_event::<SessionSetupCompleted>();
        ctx.subscribe_event::<StreamCompleted>();
        ctx.subscribe_event::<SessionPhaseChanged>();

        let csv_writer = deps.csv_path.and_then(|path| {
            BenchCsvWriter::create(&path)
                .inspect_err(|e| {
                    tracing::error!(path = %path.display(), error = %e, "failed to create CSV writer");
                })
                .ok()
        });

        // Build task lookup from all bench tasks.
        let task_lookup = tasks::bench_tasks()
            .into_iter()
            .map(|t| (t.name.to_owned(), t))
            .collect();

        let plan = deps.plan;

        let mut actor = Self {
            state: deps.state,
            pending: HashMap::new(),
            csv_writer,
            task_lookup,
            plan,
            current_pair_index: 0,
        };

        // If we have a plan, start the first pair immediately.
        if actor.plan.is_some() {
            actor.start_next_pair(ctx);
        }

        actor
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::SessionSetupCompleted(payload)) => {
                self.handle_session_setup_completed(&payload, ctx);
            }
            ActorEnvelope::Event(Event::StreamCompleted(payload)) => {
                self.handle_stream_completed(&payload, ctx);
            }
            ActorEnvelope::Event(Event::SessionPhaseChanged(payload)) => {
                self.handle_session_phase_changed(&payload, ctx).await;
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::System(_)
            | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl BenchActor {
    /// Start the next pair in the plan.
    ///
    /// Creates a new session in `AppState` with the correct model and
    /// lifecycle name, applies the user's saved preferences, and dispatches
    /// the initial prompt for the current pair in the plan.
    ///
    /// No-ops if there is no plan or all pairs have been started.
    fn start_next_pair(&mut self, ctx: &ActorContext) {
        let Some(ref plan) = self.plan else {
            return;
        };

        if self.current_pair_index >= plan.pairs.len() {
            return;
        }

        #[expect(clippy::expect_used, reason = "index verified above")]
        let (task_name, model) = plan
            .pairs
            .get(self.current_pair_index)
            .expect("index checked above");
        self.current_pair_index += 1;

        let session_id = {
            let mut state = self.state.write();

            // Use preferences for token budget, etc.
            let persona_name = state
                .context
                .active_persona
                .as_ref()
                .map_or_else(|| "coding-assistant".to_owned(), |p| p.name.clone());
            let mut new_session = ChatSessionState::new_with_profile(SessionProfile::new(
                model.clone(),
                persona_name,
                std::collections::HashSet::new(),
                std::collections::HashSet::new(),
            ));
            new_session.set_lifecycle_name(Some(task_name.clone()));

            let new_id = new_session.session_id().clone();
            state.session.insert(new_session);
            state.session.set_active(new_id.clone());
            new_id
        };

        tracing::info!(
            session_id = %session_id,
            task = %task_name,
            model = %model,
            "bench actor starting pair"
        );

        // Emit setup commands.
        let lifecycle_command = LifecycleCommand::Builtin(BuiltinId(task_name.clone()));
        let _ = ctx.send_command(Command::PersistSession(
            jinn_domain::feat::session_lifecycle::protocol::command::PersistSession {
                session_id: session_id.clone(),
            },
        ));
        let _ = ctx.send_command(Command::PushChatEntry(
            jinn_domain::feat::chat_input::protocol::command::PushChatEntry {
                session_id: session_id.clone(),
                entry: setup_running_msg(),
            },
        ));
        let _ = ctx.send_command(Command::RunSessionSetup(RunSessionSetup {
            session_id: session_id.clone(),
            command: task_name.clone(),
            args: vec![],
            lifecycle_command: Some(lifecycle_command),
        }));
    }

    /// Enqueue a message for the given session.
    fn enqueue_message(session_id: &SessionId, message: &str, ctx: &ActorContext) {
        let _ = ctx.send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
            session_id: session_id.clone(),
            entry: jinn_domain::ChatEntry::user(message.to_owned()),
        }));
    }

    /// Handle setup errors for bench sessions: record a failure row and advance.
    fn handle_setup_error(&mut self, session_id: &SessionId, error: &str, ctx: &ActorContext) {
        // Check if this session has a bench lifecycle name.
        let task_name = {
            let state = self.state.read();
            let Some(session) = state.session.get(session_id) else {
                return;
            };
            session.lifecycle_name().map(str::to_owned)
        };

        let Some(task_name) = task_name else {
            return;
        };

        // Only handle if it's a known bench task.
        let Some(task) = self.task_lookup.get(&task_name) else {
            return;
        };

        let model = {
            let state = self.state.read();
            state
                .session
                .get(session_id)
                .map(|session| session.profile().model.clone())
                .unwrap_or_default()
        };

        tracing::warn!(
            session_id = %session_id,
            task = %task_name,
            error = %error,
            "bench setup failed, recording failure"
        );

        let result = BenchResult {
            name: task_name.clone(),
            category: task.category.to_owned(),
            model,
            turns: 0,
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            wall_time_ms: 0,
            passed: false,
            status: "setup-failed".to_owned(),
        };

        // Write CSV row if writer is available.
        if let Some(ref mut writer) = self.csv_writer
            && let Err(e) = writer.write_row(&result)
        {
            tracing::error!(error = %e, "failed to write CSV row");
        }

        // Push a failure chat entry.
        let msg = format!("❌ Setup failed: {error}");
        let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
            session_id: session_id.clone(),
            entry: ChatEntry::system(msg),
        }));

        // Advance to the next pair.
        self.start_next_pair(ctx);

        // If no more pending sessions and no more pairs, signal completion.
        if self.pending.is_empty() {
            let has_more = self
                .plan
                .as_ref()
                .is_some_and(|p| self.current_pair_index < p.pairs.len());

            if !has_more {
                tracing::info!("all bench sessions completed, signaling quit");
                let mut state = self.state.write();
                state.frontend.should_quit = true;
            }
        }
    }

    /// Handle `SessionSetupCompleted` - start tracking if this is a bench session.
    fn handle_session_setup_completed(
        &mut self,
        payload: &SessionSetupCompleted,
        ctx: &ActorContext,
    ) {
        // Handle setup errors: record failure and advance to next pair.
        if let Some(ref error) = payload.error {
            self.handle_setup_error(&payload.session_id, error, ctx);
            return;
        }

        // Check if this session has a bench lifecycle name.
        let task_name = {
            let state = self.state.read();
            let Some(session) = state.session.get(&payload.session_id) else {
                return;
            };
            session.lifecycle_name().map(str::to_owned)
        };

        let Some(task_name) = task_name else {
            return;
        };

        // Only track if it's a known bench task.
        let Some(task) = self.task_lookup.get(&task_name) else {
            return;
        };

        let now = Instant::now();
        let deadline = now + task.timeout;
        let total_messages = task.messages.len();

        tracing::info!(
            session_id = %payload.session_id,
            task = %task_name,
            timeout_secs = task.timeout.as_secs(),
            messages = total_messages,
            "bench actor tracking session"
        );

        self.pending.insert(
            payload.session_id.clone(),
            BenchSession {
                task_name: task_name.clone(),
                start_time: now,
                deadline,
                verify: task.verify,
                messages_remaining: total_messages,
                next_message_index: 0,
            },
        );

        // Enqueue the first message for this session.
        if let Some(first_message) = task.messages.first() {
            Self::enqueue_message(&payload.session_id, first_message, ctx);
            if let Some(tracked) = self.pending.get_mut(&payload.session_id) {
                tracked.messages_remaining -= 1;
                tracked.next_message_index += 1;
            }
        }
    }

    /// Handle `StreamCompleted` - check timeout for tracked sessions.
    fn handle_stream_completed(&mut self, payload: &StreamCompleted, ctx: &ActorContext) {
        let Some(tracked) = self.pending.get(&payload.session_id) else {
            return;
        };

        // Check if we've exceeded the deadline.
        if Instant::now() <= tracked.deadline {
            return;
        }

        tracing::warn!(
            session_id = %payload.session_id,
            task = %tracked.task_name,
            "bench session timed out, sending CancelStream"
        );

        let _ = ctx.send_command(Command::CancelStream(CancelStream {
            session_id: payload.session_id.clone(),
        }));
    }

    /// Handle `SessionPhaseChanged` - finalize result when tracked session returns to Idle.
    #[expect(
        clippy::unused_async,
        reason = "called via .await from the async handle method"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "linear lifecycle handler orchestrating teardown, archive, cleanup phases"
    )]
    async fn handle_session_phase_changed(
        &mut self,
        payload: &SessionPhaseChanged,
        ctx: &ActorContext,
    ) {
        // Only care about Idle transitions for tracked sessions.
        if payload.new_phase != PhaseKind::Idle {
            return;
        }

        let Some(tracked) = self.pending.get(&payload.session_id) else {
            return;
        };

        // If there are more messages to send, enqueue the next one.
        if tracked.messages_remaining > 0 {
            let task_name = tracked.task_name.clone();
            let next_index = tracked.next_message_index;

            if let Some((message, task)) = self
                .task_lookup
                .get(&task_name)
                .and_then(|task| task.messages.get(next_index).map(|m| (m, task)))
            {
                // We don't use `task` but need it for the `and_then` chain.
                let _ = task;
                Self::enqueue_message(&payload.session_id, message, ctx);
                if let Some(tracked) = self.pending.get_mut(&payload.session_id) {
                    tracked.messages_remaining -= 1;
                    tracked.next_message_index += 1;
                }
                return;
            }
        }

        // All messages sent - finalize the result.
        #[expect(clippy::expect_used, reason = "existence verified above")]
        let tracked = self
            .pending
            .remove(&payload.session_id)
            .expect("session was checked above");

        let elapsed = tracked.start_time.elapsed();
        let wall_time_ms = elapsed.as_millis() as u64;

        // Read token stats and model from state.
        let (token_stats, model, cwd, cost) = {
            let state = self.state.read();
            let Some(session) = state.session.get(&payload.session_id) else {
                tracing::warn!(
                    session_id = %payload.session_id,
                    "bench session disappeared before result could be recorded"
                );
                return;
            };
            let token_summary = TokenStats::from_ledger(session.token_ledger());
            let cost = TokenStats::total_cost(session.token_ledger());
            let model = session.profile().model.clone();
            let cwd = session.cwd().to_owned();
            (token_summary, model, cwd, cost)
        };

        // Run verification.
        let report = (tracked.verify)(&cwd);
        let passed = report.passed();

        // Log each check result.
        for check in &report.checks {
            if check.passed {
                tracing::info!(
                    task = %tracked.task_name,
                    check = %check.name,
                    "check passed"
                );
            } else {
                tracing::warn!(
                    task = %tracked.task_name,
                    check = %check.name,
                    detail = %check.detail,
                    "check failed"
                );
            }
        }

        // Determine status.
        let is_timeout = Instant::now() > tracked.deadline;
        let status = if is_timeout {
            "timeout".to_owned()
        } else {
            "completed".to_owned()
        };

        // Push per-check results as system chat entries.
        for check in &report.checks {
            let msg = if check.passed {
                format!("✅ {}", check.name)
            } else {
                format!("❌ {}: {}", check.name, check.detail)
            };
            let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: payload.session_id.clone(),
                entry: ChatEntry::system(msg),
            }));
        }

        // Push summary line.
        let summary = if report.passed() {
            format!("✅ Evaluation passed - {} checks", report.checks.len())
        } else {
            let fail_count = report.failures().count();
            format!(
                "❌ Evaluation failed - {}/{} checks failed",
                fail_count,
                report.checks.len()
            )
        };
        let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
            session_id: payload.session_id.clone(),
            entry: ChatEntry::system(summary),
        }));

        let category = self
            .task_lookup
            .get(&tracked.task_name)
            .map_or("unknown", |t| t.category)
            .to_owned();

        let result = BenchResult {
            name: tracked.task_name.clone(),
            category,
            model,
            turns: u32::try_from(token_stats.request_count).unwrap_or(u32::MAX),
            tokens_in: token_stats.total_sent,
            tokens_out: token_stats.total_received,
            cost,
            wall_time_ms,
            passed,
            status,
        };

        tracing::info!(
            task = %result.name,
            model = %result.model,
            tokens_in = result.tokens_in,
            tokens_out = result.tokens_out,
            cost = result.cost,
            wall_time_ms = result.wall_time_ms,
            passed = result.passed,
            status = %result.status,
            "bench result recorded"
        );

        // Write CSV row if writer is available.
        if let Some(ref mut writer) = self.csv_writer
            && let Err(e) = writer.write_row(&result)
        {
            tracing::error!(error = %e, "failed to write CSV row");
        }

        // Advance to the next pair.
        self.start_next_pair(ctx);

        // If no more pending sessions and no more pairs, signal completion.
        if self.pending.is_empty() {
            let has_more = self
                .plan
                .as_ref()
                .is_some_and(|p| self.current_pair_index < p.pairs.len());

            if !has_more {
                tracing::info!("all bench sessions completed, signaling quit");
                let mut state = self.state.write();
                state.frontend.should_quit = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use std::sync::Arc;
    use std::time::Duration;

    use jinn_domain::RecordingSink;

    use super::*;
    use crate::orchestrator::build_plan;

    /// Create a minimal test state with a session that has a bench lifecycle.
    fn test_state_with_session() -> (State, SessionId) {
        test_state_with_named_session("hello-world")
    }

    fn test_state_with_noop_session() -> (State, SessionId) {
        test_state_with_named_session("test-noop")
    }

    fn test_state_with_named_session(lifecycle_name: &str) -> (State, SessionId) {
        let state = State::new(jinn_domain::AppState::default());
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.set_lifecycle_name(Some(lifecycle_name.to_owned()));
            s.session.active_session_id().clone()
        };
        (state, session_id)
    }

    fn test_context() -> (Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("bench-actor-test", sink.clone());
        (sink, ctx)
    }

    #[test]
    fn bench_session_is_tracked_on_setup_completed() {
        // Given a bench actor.
        let (state, session_id) = test_state_with_session();
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        // When SessionSetupCompleted fires for the bench session.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: None,
            },
            &ctx,
        );

        // Then the session is tracked.
        assert!(actor.pending.contains_key(&session_id));
        let tracked = actor.pending.get(&session_id).expect("tracked");
        assert_eq!(tracked.task_name, "hello-world");
    }

    #[test]
    fn non_bench_session_is_not_tracked() {
        // Given a bench actor with a session that has no lifecycle.
        let state = State::new(jinn_domain::AppState::default());
        let session_id = {
            let s = state.read();
            s.session.active_session_id().clone()
        };
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        // When SessionSetupCompleted fires for a non-bench session.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: None,
            },
            &ctx,
        );

        // Then the session is NOT tracked.
        assert!(!actor.pending.contains_key(&session_id));
    }

    #[test]
    fn setup_with_error_is_not_tracked() {
        // Given a bench actor with a bench session.
        let (state, session_id) = test_state_with_session();
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        // When SessionSetupCompleted fires with an error.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: Some("something failed".to_owned()),
            },
            &ctx,
        );

        // Then the session is NOT tracked.
        assert!(!actor.pending.contains_key(&session_id));
    }

    #[test]
    fn setup_failure_records_failure_csv_row() {
        // Given a bench actor with a CSV writer and a plan with 2 tasks.
        let plan = build_plan(
            &["test-model".to_owned()],
            &["hello-world".to_owned(), "json-parser".to_owned()],
        )
        .expect("plan");
        let (state, session_id) = test_state_with_session();
        let csv_dir = tempfile::TempDir::new().expect("temp dir");
        let csv_path = csv_dir.path().join("results.csv");
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: Some(csv_path.clone()),
                plan: Some(plan),
            },
            &mut ActorContext::new("test", sink.clone()),
        );
        // Activate already started the first pair - index is at 1.
        assert_eq!(actor.current_pair_index, 1);

        // When SessionSetupCompleted fires with an error for a bench session.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: Some("fixture not found".to_owned()),
            },
            &ctx,
        );

        // Then the session is NOT tracked (still no pending entry).
        assert!(!actor.pending.contains_key(&session_id));

        // And a CSV row with "setup-failed" was written.
        let content = std::fs::read_to_string(&csv_path).expect("read csv");
        let lines: Vec<&str> = content.lines().collect();
        assert!(
            lines.len() >= 2,
            "expected header + at least 1 row, got: {content}"
        );
        assert!(
            lines[1].contains("hello-world"),
            "row should contain task name"
        );
        assert!(
            lines[1].contains("setup-failed"),
            "row should contain setup-failed status"
        );
        assert!(
            lines[1].contains("false"),
            "row should contain passed=false"
        );

        // And the next pair was started (index advanced).
        assert_eq!(
            actor.current_pair_index, 2,
            "start_next_pair should have been called"
        );
    }

    #[test]
    fn setup_failure_with_no_plan_does_not_advance() {
        // Given a bench actor with no plan.
        let (state, session_id) = test_state_with_session();
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        // When SessionSetupCompleted fires with an error for a bench session.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: Some("fixture not found".to_owned()),
            },
            &ctx,
        );

        // Then no crash and the session is not tracked.
        assert!(!actor.pending.contains_key(&session_id));
    }

    #[tokio::test]
    async fn result_is_recorded_on_idle_phase_change() {
        // Given a tracked bench session with a noop task (avoids cargo check).
        let (state, session_id) = test_state_with_noop_session();

        let csv_dir = tempfile::TempDir::new().expect("temp dir");
        let csv_path = csv_dir.path().join("results.csv");

        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state: state.clone(),
                csv_path: Some(csv_path.clone()),
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        // Track the session.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: None,
            },
            &ctx,
        );

        // When SessionPhaseChanged fires with Idle.
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then the session is removed from pending.
        assert!(!actor.pending.contains_key(&session_id));

        // And the CSV file has a row (header + 1 result).
        let content = std::fs::read_to_string(&csv_path).expect("read csv");
        let lines: Vec<&str> = content.lines().collect();
        assert!(
            lines.len() >= 2,
            "expected header + at least 1 row, got: {content}"
        );
        assert!(
            lines[1].contains("test-noop"),
            "row should contain task name"
        );
    }

    #[tokio::test]
    async fn non_idle_phase_change_does_not_record() {
        // Given a tracked bench session.
        let (state, session_id) = test_state_with_session();
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: None,
            },
            &ctx,
        );

        // When SessionPhaseChanged fires with Sending (not Idle).
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Sending,
                },
                &ctx,
            )
            .await;

        // Then the session is still tracked (not finalized).
        assert!(actor.pending.contains_key(&session_id));
    }

    #[test]
    fn timeout_sends_cancel_stream() {
        // Given a tracked bench session with a very short timeout.
        let (state, session_id) = test_state_with_session();
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink.clone()),
        );

        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: None,
            },
            &ctx,
        );

        // Manually set the deadline to the past to simulate timeout.
        if let Some(tracked) = actor.pending.get_mut(&session_id) {
            tracked.deadline = Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or(Instant::now());
        }

        // When StreamCompleted fires.
        actor.handle_stream_completed(
            &StreamCompleted {
                session_id: session_id.clone(),
                reason:
                    jinn_domain::feat::provider::protocol::event::StreamCompletedReason::Finished,
                assistant_content: None,
                tool_calls: None,
                cost: None,
                provider_completion_tokens: None,
                thinking_content: None,
                dispatched_at: jiff::Timestamp::now(),
            },
            &ctx,
        );

        // Then CancelStream was sent.
        let commands = sink.commands();
        let found = commands.iter().any(|cmd| {
            matches!(
                cmd,
                Command::CancelStream(CancelStream { session_id: sid }) if sid == &session_id
            )
        });
        assert!(found, "expected CancelStream command for timed-out session");
    }

    #[test]
    fn first_message_enqueued_on_setup_completed() {
        // Given a bench actor with a tracked session.
        let (state, session_id) = test_state_with_session();
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink.clone()),
        );

        // When SessionSetupCompleted fires for the bench session.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: None,
            },
            &ctx,
        );

        // Then the first message was enqueued via EnqueueUserMessage.
        let commands = sink.commands();
        let found = commands.iter().any(|cmd| {
            matches!(
                cmd,
                Command::EnqueueUserMessage(EnqueueUserMessage { session_id: sid, .. })
                if sid == &session_id
            )
        });
        assert!(found, "expected EnqueueUserMessage for session");

        // And messages_remaining is decremented.
        let tracked = actor.pending.get(&session_id).expect("tracked");
        assert_eq!(
            tracked.messages_remaining, 0,
            "hello-world has 1 message, should be 0 remaining"
        );
    }

    #[tokio::test]
    async fn multi_message_task_enqueues_all_messages_before_finalizing() {
        // Given a bench actor with a tracked session for redirect-change-color (2 messages).
        let state = State::new(jinn_domain::AppState::default());
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.set_lifecycle_name(Some("redirect-change-color".to_owned()));
            s.session.active_session_id().clone()
        };

        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink.clone()),
        );

        // When SessionSetupCompleted fires.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: None,
            },
            &ctx,
        );

        // Then messages_remaining is 1 (first of 2 already sent).
        let tracked = actor.pending.get(&session_id).expect("tracked");
        assert_eq!(tracked.messages_remaining, 1);
        assert_eq!(tracked.next_message_index, 1);

        // When SessionPhaseChanged fires with Idle (intermediate).
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then the session is still tracked (not yet finalized).
        assert!(
            actor.pending.contains_key(&session_id),
            "session should still be pending after intermediate idle"
        );

        // And messages_remaining is now 0.
        let tracked = actor.pending.get(&session_id).expect("tracked");
        assert_eq!(tracked.messages_remaining, 0);

        // When SessionPhaseChanged fires with Idle again (final).
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then the session is removed from pending (finalized).
        assert!(
            !actor.pending.contains_key(&session_id),
            "session should be removed after final idle"
        );
    }

    #[test]
    fn plan_driven_actor_starts_first_pair_on_activate() {
        // Given a plan with 1 model and 1 task.
        let plan =
            build_plan(&["test-model".to_owned()], &["hello-world".to_owned()]).expect("plan");

        let state = State::new(jinn_domain::AppState::default());
        let (sink, _ctx) = test_context();
        let mut ctx = ActorContext::new("test", sink.clone());

        // When activating the bench actor with the plan.
        let actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: Some(plan),
            },
            &mut ctx,
        );

        // Then the current_pair_index has advanced to 1.
        assert_eq!(actor.current_pair_index, 1);

        // And RunSessionSetup was emitted.
        let commands = sink.commands();
        let found = commands.iter().any(|cmd| {
            matches!(cmd, Command::RunSessionSetup(RunSessionSetup {
                command,
                lifecycle_command: Some(LifecycleCommand::Builtin(BuiltinId(id))),
                ..
            }) if command == "hello-world" && id == "hello-world")
        });
        assert!(found, "expected RunSessionSetup for hello-world");
    }

    #[tokio::test]
    async fn success_pushes_checkmark_chat_entry() {
        // Given a tracked bench session in a directory that passes hello-world verification.
        let work_dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::create_dir_all(work_dir.path().join("src")).expect("create src dir");
        std::fs::write(
            work_dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");
        std::fs::write(work_dir.path().join("src/main.rs"), "fn main() {}")
            .expect("write src/main.rs");

        let state = State::new(jinn_domain::AppState::default());
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.set_lifecycle_name(Some("hello-world".to_owned()));
            session.set_cwd(work_dir.path().to_owned());
            s.session.active_session_id().clone()
        };

        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink.clone()),
        );

        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: work_dir.path().to_owned(),
                error: None,
            },
            &ctx,
        );

        // When SessionPhaseChanged fires with Idle.
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then a PushChatEntry with "Evaluation passed" was sent.
        let commands = sink.commands();
        let found = commands.iter().any(|cmd| {
            matches!(
                cmd,
                Command::PushChatEntry(PushChatEntry { session_id: sid, entry })
                if sid == &session_id
                    && matches!(&entry.kind, jinn_domain::ChatEntryKind::System(t) if t.contains("Evaluation passed"))
            )
        });
        assert!(
            found,
            "expected PushChatEntry with 'Evaluation passed' for passing session"
        );
    }

    #[tokio::test]
    async fn failure_pushes_x_chat_entries_with_details() {
        // Given a tracked bench session whose CWD has no files (verify will fail).
        let state = State::new(jinn_domain::AppState::default());

        // Use a temp dir so verification runs against an empty directory.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.set_lifecycle_name(Some("hello-world".to_owned()));
            // Set CWD to the temp dir - verification will fail because no files exist.
            session.set_cwd(temp_dir.path().to_owned());
            s.session.active_session_id().clone()
        };

        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink.clone()),
        );

        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: temp_dir.path().to_owned(),
                error: None,
            },
            &ctx,
        );

        // When SessionPhaseChanged fires with Idle.
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then a PushChatEntry with "❌" was sent.
        let commands = sink.commands();
        let failure_summary = commands.iter().any(|cmd| {
            matches!(
                cmd,
                Command::PushChatEntry(PushChatEntry { session_id: sid, entry })
                if sid == &session_id
                    && matches!(&entry.kind, jinn_domain::ChatEntryKind::System(t) if t.contains("Evaluation failed"))
            )
        });
        assert!(
            failure_summary,
            "expected PushChatEntry with ❌ for failing session"
        );

        // And at least one detail entry with "•" was sent.
        let detail_entry = commands.iter().any(|cmd| {
            matches!(
                cmd,
                Command::PushChatEntry(PushChatEntry { session_id: sid, entry })
                if sid == &session_id
                    && matches!(&entry.kind, jinn_domain::ChatEntryKind::System(t) if t.contains("file_exists"))
            )
        });
        assert!(
            detail_entry,
            "expected PushChatEntry with • detail for failing session"
        );
    }

    #[test]
    fn setup_error_sets_should_quit_when_no_more_pairs() {
        // Given a plan with exactly 1 task/model pair.
        let plan =
            build_plan(&["test-model".to_owned()], &["hello-world".to_owned()]).expect("plan");
        let (state, session_id) = test_state_with_session();
        let csv_dir = tempfile::TempDir::new().expect("temp dir");
        let csv_path = csv_dir.path().join("results.csv");
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: Some(csv_path),
                plan: Some(plan),
            },
            &mut ActorContext::new("test", sink),
        );
        // Activate started the first pair - index is at 1, plan has 1 pair.
        assert_eq!(actor.current_pair_index, 1);

        // When SessionSetupCompleted fires with an error for the bench session.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: Some("fixture not found".to_owned()),
            },
            &ctx,
        );

        // Then should_quit is true (index 1 == plan len 1, so no more pairs).
        // This kills: < vs ==, < vs <=, < vs >, and delete ! on has_more.
        let state = actor.state.read();
        assert!(
            state.frontend.should_quit,
            "expected should_quit when all pairs are exhausted after setup error"
        );
    }

    #[test]
    fn setup_error_does_not_quit_when_more_pairs_remain() {
        // Given a plan with 3 tasks.
        let plan = build_plan(
            &["test-model".to_owned()],
            &[
                "hello-world".to_owned(),
                "json-parser".to_owned(),
                "redirect-change-color".to_owned(),
            ],
        )
        .expect("plan");
        let (state, session_id) = test_state_with_session();
        let csv_dir = tempfile::TempDir::new().expect("temp dir");
        let csv_path = csv_dir.path().join("results.csv");
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: Some(csv_path),
                plan: Some(plan),
            },
            &mut ActorContext::new("test", sink),
        );
        // Activate started pair 0 - index is at 1, plan has 3 pairs.
        assert_eq!(actor.current_pair_index, 1);

        // When SessionSetupCompleted fires with an error.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: Some("fixture not found".to_owned()),
            },
            &ctx,
        );

        // Then should_quit is false (index 2 < plan len 3, more pairs remain).
        // This kills: delete ! on has_more (if deleted, has_more=true would enter block incorrectly).
        let state = actor.state.read();
        assert!(
            !state.frontend.should_quit,
            "expected should_quit=false when more pairs remain after setup error"
        );
        assert_eq!(
            actor.current_pair_index, 2,
            "should have advanced to next pair"
        );
    }

    #[test]
    fn deadline_is_in_future_after_setup_completed() {
        // Given a bench actor that tracks a session.
        let (state, session_id) = test_state_with_session();
        let (sink, ctx) = test_context();
        let before = Instant::now();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: None,
            },
            &ctx,
        );

        // Then the tracked session's deadline is in the future.
        // This kills: replace + with - (deadline would be in the past).
        let tracked = actor.pending.get(&session_id).expect("tracked");
        assert!(
            tracked.deadline > before,
            "deadline should be in the future (now + timeout), got {:?}",
            tracked.deadline
        );
    }

    #[tokio::test]
    async fn timeout_status_set_when_deadline_exceeded() {
        // Given a tracked bench session whose deadline has already passed.
        let work_dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::create_dir_all(work_dir.path().join("src")).expect("create src dir");
        std::fs::write(
            work_dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");
        std::fs::write(work_dir.path().join("src/main.rs"), "fn main() {}")
            .expect("write src/main.rs");

        let state = State::new(jinn_domain::AppState::default());
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.set_lifecycle_name(Some("hello-world".to_owned()));
            session.set_cwd(work_dir.path().to_owned());
            s.session.active_session_id().clone()
        };

        let csv_dir = tempfile::TempDir::new().expect("temp dir");
        let csv_path = csv_dir.path().join("results.csv");
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: Some(csv_path.clone()),
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: work_dir.path().to_owned(),
                error: None,
            },
            &ctx,
        );

        // Set deadline to the past to simulate timeout.
        if let Some(tracked) = actor.pending.get_mut(&session_id) {
            tracked.deadline = Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or(Instant::now());
        }

        // When SessionPhaseChanged fires with Idle.
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then the CSV row has status "timeout".
        // This kills: > vs ==, > vs <, > vs >= on the Instant::now() > deadline check.
        let content = std::fs::read_to_string(&csv_path).expect("read csv");
        assert!(
            content.contains("timeout"),
            "expected 'timeout' status in CSV when deadline exceeded, got: {content}"
        );
    }

    #[tokio::test]
    async fn completed_status_set_when_before_deadline() {
        // Given a tracked bench session whose deadline is far in the future.
        let work_dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::create_dir_all(work_dir.path().join("src")).expect("create src dir");
        std::fs::write(
            work_dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");
        std::fs::write(work_dir.path().join("src/main.rs"), "fn main() {}")
            .expect("write src/main.rs");

        let state = State::new(jinn_domain::AppState::default());
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.set_lifecycle_name(Some("hello-world".to_owned()));
            session.set_cwd(work_dir.path().to_owned());
            s.session.active_session_id().clone()
        };

        let csv_dir = tempfile::TempDir::new().expect("temp dir");
        let csv_path = csv_dir.path().join("results.csv");
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: Some(csv_path.clone()),
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: work_dir.path().to_owned(),
                error: None,
            },
            &ctx,
        );

        // Set deadline to far future - should NOT be timeout.
        if let Some(tracked) = actor.pending.get_mut(&session_id) {
            tracked.deadline = Instant::now() + Duration::from_secs(3600);
        }

        // When SessionPhaseChanged fires with Idle.
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then the CSV row has status "completed", not "timeout".
        // This kills: > vs >= (at exact boundary, >= would make everything timeout).
        let content = std::fs::read_to_string(&csv_path).expect("read csv");
        assert!(
            content.contains("completed"),
            "expected 'completed' status when before deadline, got: {content}"
        );
        assert!(
            !content.contains("timeout"),
            "should not have 'timeout' status when before deadline"
        );
    }

    #[tokio::test]
    async fn intermediate_idle_decrements_counters_correctly() {
        // Given a tracked bench session for a multi-message task (redirect-change-color has 2 msgs).
        let state = State::new(jinn_domain::AppState::default());
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.set_lifecycle_name(Some("redirect-change-color".to_owned()));
            s.session.active_session_id().clone()
        };

        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state,
                csv_path: None,
                plan: None,
            },
            &mut ActorContext::new("test", sink),
        );

        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: PathBuf::from("/tmp"),
                error: None,
            },
            &ctx,
        );

        // After setup, first message was sent. Verify initial counter state.
        let tracked = actor.pending.get(&session_id).expect("tracked");
        assert_eq!(
            tracked.messages_remaining, 1,
            "should have 1 remaining after first send"
        );
        assert_eq!(
            tracked.next_message_index, 1,
            "index should be 1 after first send"
        );

        // When SessionPhaseChanged fires with Idle (intermediate - more messages left).
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then counters were correctly updated (not multiplied or negated).
        // This kills: += with -=, += with *= on both messages_remaining and next_message_index.
        let tracked = actor.pending.get(&session_id).expect("still tracked");
        assert_eq!(
            tracked.messages_remaining, 0,
            "messages_remaining should be 0 (was 1, decremented by 1), not multiplied or incremented"
        );
        assert_eq!(
            tracked.next_message_index, 2,
            "next_message_index should be 2 (was 1, incremented by 1), not multiplied or decremented"
        );
    }

    #[tokio::test]
    async fn finalization_sets_should_quit_when_plan_exhausted() {
        // Given a plan with exactly 1 task.
        let plan =
            build_plan(&["test-model".to_owned()], &["hello-world".to_owned()]).expect("plan");

        let work_dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::create_dir_all(work_dir.path().join("src")).expect("create src dir");
        std::fs::write(
            work_dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");
        std::fs::write(work_dir.path().join("src/main.rs"), "fn main() {}")
            .expect("write src/main.rs");

        let state = State::new(jinn_domain::AppState::default());
        // We need to set the session lifecycle AFTER activate because activate
        // creates a new session for the first pair. We'll track that session.
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state: state.clone(),
                csv_path: None,
                plan: Some(plan),
            },
            &mut ActorContext::new("test", sink.clone()),
        );

        // Find the session that was created by start_next_pair.
        let session_id = {
            let s = state.read();
            s.session.active_session_id().clone()
        };

        // Set lifecycle and CWD so the actor tracks it and verification passes.
        {
            let mut s = state.write();
            let session = s.session.active_session_mut();
            session.set_lifecycle_name(Some("hello-world".to_owned()));
            session.set_cwd(work_dir.path().to_owned());
        }

        // Fire setup completed to track the session.
        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: work_dir.path().to_owned(),
                error: None,
            },
            &ctx,
        );

        // Set deadline to future so status is "completed".
        if let Some(tracked) = actor.pending.get_mut(&session_id) {
            tracked.deadline = Instant::now() + Duration::from_secs(3600);
        }

        // When SessionPhaseChanged fires with Idle (finalizes the session).
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then should_quit is set because the plan is exhausted.
        // This kills: < vs ==, < vs >, < vs <= on current_pair_index < plan len,
        // and delete ! on has_more.
        let s = state.read();
        assert!(
            s.frontend.should_quit,
            "expected should_quit when all pairs finalized and plan exhausted"
        );
    }

    #[tokio::test]
    async fn finalization_does_not_quit_when_more_pairs_remain() {
        // Given a plan with 3 tasks.
        let plan = build_plan(
            &["test-model".to_owned()],
            &[
                "hello-world".to_owned(),
                "json-parser".to_owned(),
                "redirect-change-color".to_owned(),
            ],
        )
        .expect("plan");

        let work_dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::create_dir_all(work_dir.path().join("src")).expect("create src dir");
        std::fs::write(
            work_dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");
        std::fs::write(work_dir.path().join("src/main.rs"), "fn main() {}")
            .expect("write src/main.rs");

        let state = State::new(jinn_domain::AppState::default());
        let (sink, ctx) = test_context();
        let mut actor = BenchActor::activate(
            BenchActorDeps {
                state: state.clone(),
                csv_path: None,
                plan: Some(plan),
            },
            &mut ActorContext::new("test", sink.clone()),
        );

        // Find the session created by start_next_pair.
        let session_id = {
            let s = state.read();
            s.session.active_session_id().clone()
        };

        {
            let mut s = state.write();
            let session = s.session.active_session_mut();
            session.set_lifecycle_name(Some("hello-world".to_owned()));
            session.set_cwd(work_dir.path().to_owned());
        }

        actor.handle_session_setup_completed(
            &SessionSetupCompleted {
                session_id: session_id.clone(),
                cwd: work_dir.path().to_owned(),
                error: None,
            },
            &ctx,
        );

        if let Some(tracked) = actor.pending.get_mut(&session_id) {
            tracked.deadline = Instant::now() + Duration::from_secs(3600);
        }

        // When SessionPhaseChanged fires with Idle.
        actor
            .handle_session_phase_changed(
                &SessionPhaseChanged {
                    session_id: session_id.clone(),
                    old_phase: PhaseKind::Streaming,
                    new_phase: PhaseKind::Idle,
                },
                &ctx,
            )
            .await;

        // Then should_quit is NOT set because there's still 1 more pair.
        // This kills: delete ! on has_more (if deleted, has_more=true would set should_quit incorrectly).
        let s = state.read();
        assert!(
            !s.frontend.should_quit,
            "expected should_quit=false when more pairs remain after finalization"
        );
    }
}
