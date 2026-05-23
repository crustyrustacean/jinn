//! Bench actor — orchestrates bench execution and records results.
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
use nullslop_domain::feat::chat_input::protocol::command::{EnqueueUserMessage, PushChatEntry};
use nullslop_domain::feat::provider::protocol::command::CancelStream;
use nullslop_domain::feat::provider::protocol::event::StreamCompleted;
use nullslop_domain::feat::session::chat_session::{ChatSessionState, SessionPhase};
use nullslop_domain::feat::session::profile::SessionProfile;
use nullslop_domain::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use nullslop_domain::feat::session::session_actor::setup_running_msg;
use nullslop_domain::feat::session::token_stats::TokenStats;
use nullslop_domain::feat::session_lifecycle::builtin::{BuiltinId, LifecycleCommand};
use nullslop_domain::feat::session_lifecycle::protocol::command::RunSessionSetup;
use nullslop_domain::feat::session_lifecycle::protocol::event::SessionSetupCompleted;
use nullslop_domain::protocol::PromptStrategyId;
use nullslop_domain::protocol::{ChatEntry, Command, Event, SessionId};
use nullslop_domain::{Actor, ActorContext, ActorEnvelope, NoDirectMsg, State};

/// A tracked bench session.
struct BenchSession {
    /// The bench task name (matches the lifecycle name).
    task_name: String,
    /// When this session was first tracked (after setup completed).
    start_time: Instant,
    /// Deadline for timeout — `start_time + task.timeout`.
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
    /// Tracked bench sessions — keyed by session ID.
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
    /// Creates a new session in AppState with the correct model and lifecycle_name,
    /// then emits setup commands. Does nothing if no plan or all pairs are done.
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

        // Create session in AppState.
        let session_id = {
            let mut state = self.state.write();

            // Use preferences for strategy, token budget, etc.
            let strategy = state
                .frontend
                .preferences
                .last_strategy
                .as_deref()
                .map_or_else(PromptStrategyId::passthrough, PromptStrategyId::new);
            let persona_name = state
                .context
                .active_persona
                .as_ref()
                .map_or_else(|| "coding-assistant".to_owned(), |p| p.name.clone());
            let token_budget = state.frontend.preferences.context_token_budget.budget;
            let sliding_window_size = state.frontend.preferences.context_sliding_window.size;

            let mut new_session = ChatSessionState::new_with_profile(SessionProfile::new(
                model.clone(),
                strategy,
                persona_name,
                token_budget,
                sliding_window_size,
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
            nullslop_domain::feat::session_lifecycle::protocol::command::PersistSession {
                session_id: session_id.clone(),
            },
        ));
        let _ = ctx.send_command(Command::PushChatEntry(
            nullslop_domain::feat::chat_input::protocol::command::PushChatEntry {
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
            entry: nullslop_domain::ChatEntry::user(message.to_owned()),
        }));
    }

    /// Handle `SessionSetupCompleted` — start tracking if this is a bench session.
    fn handle_session_setup_completed(
        &mut self,
        payload: &SessionSetupCompleted,
        ctx: &ActorContext,
    ) {
        // Skip sessions with setup errors.
        if payload.error.is_some() {
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

    /// Handle `StreamCompleted` — check timeout for tracked sessions.
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

    /// Handle `SessionPhaseChanged` — finalize result when tracked session returns to Idle.
    #[expect(
        clippy::unused_async,
        reason = "called via .await from the async handle method"
    )]
    async fn handle_session_phase_changed(
        &mut self,
        payload: &SessionPhaseChanged,
        ctx: &ActorContext,
    ) {
        // Only care about Idle transitions for tracked sessions.
        if payload.new_phase != SessionPhase::Idle {
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

        // All messages sent — finalize the result.
        #[expect(clippy::expect_used, reason = "existence verified above")]
        let tracked = self
            .pending
            .remove(&payload.session_id)
            .expect("session was checked above");

        let elapsed = tracked.start_time.elapsed();
        let wall_time_ms = elapsed.as_millis() as u64;

        // Read token stats and model from state.
        let (token_stats, model, cwd) = {
            let state = self.state.read();
            let Some(session) = state.session.get(&payload.session_id) else {
                tracing::warn!(
                    session_id = %payload.session_id,
                    "bench session disappeared before result could be recorded"
                );
                return;
            };
            let token_summary = TokenStats::from_ledger(session.token_ledger());
            let model = session.profile().model.clone();
            let cwd = session.cwd().to_owned();
            (token_summary, model, cwd)
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

        // Push evaluation results into the session as system chat entries.
        let failures: Vec<_> = report.failures().collect();
        if failures.is_empty() {
            let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: payload.session_id.clone(),
                entry: ChatEntry::system(format!(
                    "✅ Evaluation passed — {} checks",
                    report.checks.len()
                )),
            }));
        } else {
            let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: payload.session_id.clone(),
                entry: ChatEntry::system(format!(
                    "❌ Evaluation failed — {}/{} checks failed",
                    failures.len(),
                    report.checks.len()
                )),
            }));
            for failure in &failures {
                let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry: ChatEntry::system(format!("  • {}: {}", failure.name, failure.detail)),
                }));
            }
        }

        let detail = if report.passed() {
            String::new()
        } else {
            report
                .failures()
                .map(|f| format!("{}: {}", f.name, f.detail))
                .collect::<Vec<_>>()
                .join("; ")
        };

        let result = BenchResult {
            name: tracked.task_name.clone(),
            model,
            turns: u32::try_from(token_stats.request_count).unwrap_or(u32::MAX),
            tokens_in: token_stats.total_sent,
            tokens_out: token_stats.total_received,
            cost: 0.0,
            wall_time_ms,
            passed,
            status,
            detail,
        };

        tracing::info!(
            task = %result.name,
            model = %result.model,
            tokens_in = result.tokens_in,
            tokens_out = result.tokens_out,
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

    use nullslop_domain::RecordingSink;

    use super::*;
    use crate::orchestrator::build_plan;

    /// Create a minimal test state with a session that has a bench lifecycle.
    fn test_state_with_session() -> (State, SessionId) {
        let state = State::new(nullslop_domain::AppState::default());
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.set_lifecycle_name(Some("hello-world".to_owned()));
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
        let state = State::new(nullslop_domain::AppState::default());
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

    #[tokio::test]
    async fn result_is_recorded_on_idle_phase_change() {
        // Given a tracked bench session.
        let (state, session_id) = test_state_with_session();

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
                    new_phase: SessionPhase::Idle,
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
            lines[1].contains("hello-world"),
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
                    new_phase: SessionPhase::Sending,
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
                reason: nullslop_domain::feat::provider::protocol::event::StreamCompletedReason::Finished,
                assistant_content: None,
                tool_calls: None,
                cost: None,
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
        let state = State::new(nullslop_domain::AppState::default());
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
                    new_phase: SessionPhase::Idle,
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
                    new_phase: SessionPhase::Idle,
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
        let plan = build_plan(&["test-model".to_owned()], &["hello-world".to_owned()]);

        let state = State::new(nullslop_domain::AppState::default());
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

        let state = State::new(nullslop_domain::AppState::default());
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
                    new_phase: SessionPhase::Idle,
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
                    && matches!(&entry.kind, nullslop_domain::ChatEntryKind::System(t) if t.contains("Evaluation passed"))
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
        let state = State::new(nullslop_domain::AppState::default());

        // Use a temp dir so verification runs against an empty directory.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let session_id = {
            let mut s = state.write();
            let session = s.active_session_mut();
            session.set_lifecycle_name(Some("hello-world".to_owned()));
            // Set CWD to the temp dir — verification will fail because no files exist.
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
                    new_phase: SessionPhase::Idle,
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
                    && matches!(&entry.kind, nullslop_domain::ChatEntryKind::System(t) if t.contains("Evaluation failed"))
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
                    && matches!(&entry.kind, nullslop_domain::ChatEntryKind::System(t) if t.contains("file_exists"))
            )
        });
        assert!(
            detail_entry,
            "expected PushChatEntry with • detail for failing session"
        );
    }
}
