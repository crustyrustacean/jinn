// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Judge coordinator actor — orchestrates judge evaluation cycles.
//!
//! Subscribes to [`SessionPhaseChanged`] and [`JudgeVerdict`] events.
//! When an origin session transitions to `Idle`, triggers attached judges
//! by pushing a user message into each judge session. When all verdicts
//! are collected, consolidates and dispatches a single message to the origin:
//!
//! - **All passed** → system message ("✓ All judges passed")
//! - **Any failed** → user message with consolidated failure summaries
//!
//! The coordinator does **not** create, launch, or manage sessions.
//! It only pushes trigger messages and collects verdicts.

use std::collections::HashMap;
use std::fmt::Write;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::{EnqueueUserMessage, PushChatEntry};
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::chat_session::SessionPhase;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::protocol::{Command, Event, SessionId};

use super::JudgeMeta;
use super::protocol::{JudgeVerdict, Verdict};

/// Resolve the effective auto-reset value for a judge session.
///
/// If the per-session override (`JudgeMeta::auto_reset`) is set, use that.
/// Otherwise, fall back to the judge file's default (`Judge::auto_reset`)
/// looked up by name from the loaded judge definitions.
pub fn resolve_effective_auto_reset(meta: &JudgeMeta, judges: &[super::Judge]) -> bool {
    meta.auto_reset.unwrap_or_else(|| {
        judges
            .iter()
            .find(|j| j.name == meta.judge_name)
            .is_some_and(|j| j.auto_reset)
    })
}

/// Maximum number of consecutive retries before force-cancelling a judge.
const MAX_JUDGE_RETRIES: u32 = 3;

/// Per-judge tracking within a pending evaluation cycle.
#[derive(Debug)]
struct PendingJudge {
    /// The judge definition name.
    judge_name: String,
    /// Number of consecutive trigger attempts without a verdict.
    retry_count: u32,
}

/// Pending verdicts for an origin session.
#[derive(Debug, Default)]
struct PendingVerdicts {
    /// Per-judge tracking: judge session ID → its state.
    judges: HashMap<SessionId, PendingJudge>,
    /// Verdicts received so far.
    received: Vec<ReceivedVerdict>,
}

/// A single received verdict, stored until consolidation.
#[derive(Debug)]
struct ReceivedVerdict {
    /// The judge session that rendered the verdict.
    #[expect(dead_code, reason = "kept for future diagnostics")]
    judge_session_id: SessionId,
    /// The judge name.
    judge_name: String,
    /// The verdict.
    verdict: Verdict,
}

/// The judge coordinator actor.
///
/// Orchestrates the evaluation loop for judge sessions attached to an origin.
/// Does NOT create, launch, or manage sessions — only pushes trigger messages
/// and collects verdicts.
pub struct JudgeCoordinatorActor {
    /// Shared application state.
    state: State,
    /// Origin session ID → pending verdicts.
    pending: HashMap<SessionId, PendingVerdicts>,
}

/// Dependencies for [`JudgeCoordinatorActor`].
pub struct JudgeCoordinatorActorDeps {
    /// Shared application state.
    pub state: State,
}

impl Actor for JudgeCoordinatorActor {
    type Message = NoDirectMsg;
    type Deps = JudgeCoordinatorActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<SessionPhaseChanged>();
        ctx.subscribe_event::<JudgeVerdict>();
        ctx.subscribe_command::<crate::feat::judge::protocol::CancelPendingJudgeEvaluation>();
        ctx.set_description("Orchestrates judge evaluation cycles on origin Idle");
        Self {
            state: deps.state,
            pending: HashMap::new(),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::SessionPhaseChanged(ref payload)) => {
                self.handle_session_phase_changed(payload, ctx);
            }
            ActorEnvelope::Event(Event::JudgeVerdict(ref payload)) => {
                self.handle_judge_verdict(payload, ctx);
            }
            ActorEnvelope::Command(Command::CancelPendingJudgeEvaluation(ref payload)) => {
                self.handle_cancel_pending(payload, ctx);
            }
            _ => {}
        }
    }
}

impl JudgeCoordinatorActor {
    /// Handle a session phase change.
    ///
    /// When a non-judge session transitions to `Idle`:
    /// 1. Scan for attached judges targeting this origin.
    /// 2. If none found, do nothing.
    /// 3. Push a trigger user message to each attached judge session.
    /// 4. Record the expected count in the pending map.
    fn handle_session_phase_changed(&mut self, payload: &SessionPhaseChanged, ctx: &ActorContext) {
        // Only care about Idle transitions.
        if payload.new_phase != SessionPhase::Idle {
            return;
        }

        let origin_id = &payload.session_id;

        // Check if this is a judge session going Idle.
        let is_judge = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(origin_id) else {
                return;
            };
            session.is_judge()
        };
        if is_judge {
            self.handle_judge_idle(origin_id, ctx);
            return;
        }

        // Scan for attached judges.
        let attached_judges: Vec<(SessionId, String)> = {
            let guard = self.state.read();
            guard
                .session
                .iter()
                .filter_map(|(id, session)| {
                    let meta = session.judge().as_ref()?;
                    if meta.origin_session == *origin_id && meta.is_attached {
                        Some((id.clone(), meta.judge_name.clone()))
                    } else {
                        None
                    }
                })
                .collect()
        };

        if attached_judges.is_empty() {
            return;
        }

        let expected = attached_judges.len();
        tracing::info!(
            origin = %origin_id,
            count = expected,
            "origin went idle, triggering attached judges"
        );

        // Push trigger message to each attached judge session.
        for (judge_session_id, _judge_name) in &attached_judges {
            self.trigger_judge(judge_session_id, ctx);
        }

        // Push a system notification to the origin listing evaluating judges.
        let judge_names: Vec<&str> = attached_judges
            .iter()
            .map(|(_, name)| name.as_str())
            .collect();
        let notification = format!("⚖ Evaluating: {}", judge_names.join(", "));
        let notification_entry = ChatEntry::system(&notification);
        let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
            session_id: origin_id.clone(),
            entry: notification_entry,
        }));

        // Mark the origin session as busy (spinner).
        {
            let mut guard = self.state.write();
            if let Some(origin) = guard.session.get_mut(origin_id) {
                origin.mark_busy();
            }
        }

        // Record pending judges.
        let mut pending_verdicts = PendingVerdicts::default();
        for (judge_session_id, judge_name) in attached_judges {
            pending_verdicts.judges.insert(
                judge_session_id,
                PendingJudge {
                    judge_name,
                    retry_count: 0,
                },
            );
        }
        self.pending.insert(origin_id.clone(), pending_verdicts);
    }

    /// Trigger (or re-trigger) a judge evaluation.
    ///
    /// Handles auto-reset if configured, then pushes the trigger user message.
    fn trigger_judge(&self, judge_session_id: &SessionId, ctx: &ActorContext) {
        let judge_name = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(judge_session_id) else {
                return;
            };
            let Some(meta) = session.judge() else {
                return;
            };
            meta.judge_name.clone()
        };

        // Auto-reset: if the judge's effective auto-reset is true,
        // reset its history (keeping pinned entries) before triggering.
        let should_reset = {
            let guard = self.state.read();
            if let Some(judge_session) = guard.session.get(judge_session_id) {
                if let Some(meta) = judge_session.judge().as_ref() {
                    resolve_effective_auto_reset(meta, &guard.context.judges)
                } else {
                    false
                }
            } else {
                false
            }
        };
        if should_reset {
            let mut guard = self.state.write();
            if let Some(judge_session) = guard.session.get_mut(judge_session_id) {
                judge_session.reset_judge_history();
                tracing::debug!(
                    judge = %judge_name,
                    session = %judge_session_id,
                    "auto-reset judge history before trigger"
                );
            }
        }

        let trigger_text =
            String::from("The agent has completed its turn. Please evaluate it's work.");
        let trigger_entry = ChatEntry::user(trigger_text);
        let _ = ctx.send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
            session_id: judge_session_id.clone(),
            entry: trigger_entry,
        }));
        tracing::debug!(
            judge = %judge_name,
            session = %judge_session_id,
            "triggered judge evaluation"
        );
    }

    /// Handle a judge session transitioning to Idle.
    ///
    /// If this judge has a pending evaluation (is in the pending map for its origin),
    /// and has not yet submitted a verdict, increment its retry count and re-trigger.
    /// If retry count exceeds MAX_JUDGE_RETRIES, force-cancel the origin evaluation.
    fn handle_judge_idle(&mut self, judge_session_id: &SessionId, ctx: &ActorContext) {
        // Find the origin for this judge.
        let origin_id = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(judge_session_id) else {
                return;
            };
            let Some(meta) = session.judge() else {
                return;
            };
            meta.origin_session.clone()
        };

        // Check if the origin has a pending entry and this judge is still expected.
        let Some(pending) = self.pending.get_mut(&origin_id) else {
            return;
        };

        // Check if this judge already submitted a verdict.
        let already_responded = pending
            .received
            .iter()
            .any(|v| v.judge_session_id == *judge_session_id);

        if already_responded {
            // Judge already responded — this Idle is from a re-trigger that resulted
            // in a verdict being emitted. No action needed.
            return;
        }

        // Check if the judge is still in the pending list.
        let Some(judge_state) = pending.judges.get_mut(judge_session_id) else {
            return;
        };

        // Safety check: don't retry if the origin has left Idle.
        {
            let guard = self.state.read();
            if let Some(origin_session) = guard.session.get(&origin_id) {
                if origin_session.phase() != SessionPhase::Idle {
                    tracing::debug!(
                        origin = %origin_id,
                        "origin left idle during judge retry, skipping"
                    );
                    return;
                }
            } else {
                return;
            }
        }

        judge_state.retry_count += 1;

        if judge_state.retry_count < MAX_JUDGE_RETRIES {
            tracing::info!(
                judge = %judge_state.judge_name,
                session = %judge_session_id,
                retry = judge_state.retry_count,
                max = MAX_JUDGE_RETRIES,
                "judge went idle without verdict, retrying"
            );
            // Re-trigger the judge.
            self.trigger_judge(judge_session_id, ctx);
        } else {
            tracing::warn!(
                judge = %judge_state.judge_name,
                session = %judge_session_id,
                max = MAX_JUDGE_RETRIES,
                "judge exhausted retries, force-cancelling evaluation"
            );
            // Add a synthetic fail verdict (keep the judge in pending.judges so
            // it still counts toward the total expected).
            let judge_state = pending
                .judges
                .get(judge_session_id)
                .expect("just checked");
            let reason = format!(
                "Judge '{}' failed to produce a verdict after {} attempts",
                judge_state.judge_name, MAX_JUDGE_RETRIES
            );
            pending.received.push(ReceivedVerdict {
                judge_session_id: judge_session_id.clone(),
                judge_name: judge_state.judge_name.clone(),
                verdict: Verdict::Fail(reason),
            });

            // Check if all remaining judges have reported.
            if pending.received.len() >= pending.judges.len() {
                let pending = self
                    .pending
                    .remove(&origin_id)
                    .expect("just checked entry exists");
                self.consolidate_and_dispatch(&origin_id, pending, ctx);
            }
        }
    }

    /// Handle a judge verdict.
    ///
    /// 1. Look up the origin session in the pending map.
    /// 2. If not found (stale or unexpected), ignore.
    /// 3. If the origin is no longer Idle, discard pending results.
    /// 4. Append the verdict.
    /// 5. Check if all expected verdicts are in — if yes, consolidate and dispatch.
    fn handle_judge_verdict(&mut self, payload: &JudgeVerdict, ctx: &ActorContext) {
        let origin_id = &payload.origin_session_id;

        let Some(pending) = self.pending.get_mut(origin_id) else {
            tracing::debug!(
                origin = %origin_id,
                judge = %payload.judge_name,
                "received verdict for origin with no pending entry, ignoring"
            );
            return;
        };

        // Safety check: if the origin is no longer Idle, discard pending results.
        {
            let guard = self.state.read();
            if let Some(session) = guard.session.get(origin_id) {
                if session.phase() != SessionPhase::Idle {
                    tracing::warn!(
                        origin = %origin_id,
                        phase = ?session.phase(),
                        "origin left idle before all verdicts collected, discarding pending"
                    );
                    drop(guard);
                    self.pending.remove(origin_id);
                    return;
                }
            } else {
                // Origin session gone — discard.
                drop(guard);
                self.pending.remove(origin_id);
                return;
            }
        }

        // Guard: only accept verdicts from judges we're still expecting.
        if !pending.judges.contains_key(&payload.judge_session_id) {
            tracing::debug!(
                judge = %payload.judge_session_id,
                "verdict from judge not in pending list, ignoring"
            );
            return;
        }

        // Append the verdict.
        pending.received.push(ReceivedVerdict {
            judge_session_id: payload.judge_session_id.clone(),
            judge_name: payload.judge_name.clone(),
            verdict: payload.verdict.clone(),
        });

        tracing::debug!(
            origin = %origin_id,
            judge = %payload.judge_name,
            received = pending.received.len(),
            expected = pending.judges.len(),
            "received judge verdict"
        );

        // Check if all verdicts are in.
        if pending.received.len() < pending.judges.len() {
            return;
        }

        // Consolidate and dispatch.
        let pending = self
            .pending
            .remove(origin_id)
            .expect("just checked entry exists");

        self.consolidate_and_dispatch(origin_id, pending, ctx);
    }

    /// Consolidate all verdicts and dispatch to the origin session.
    ///
    /// - All passed → `PushChatEntry` system message to origin.
    /// - Any failed → `EnqueueUserMessage` user message with consolidated summary.
    #[allow(clippy::unused_self, clippy::needless_pass_by_value)]
    fn consolidate_and_dispatch(
        &self,
        origin_id: &SessionId,
        pending: PendingVerdicts,
        ctx: &ActorContext,
    ) {
        // Clear the busy spinner on the origin session.
        {
            let mut guard = self.state.write();
            if let Some(origin) = guard.session.get_mut(origin_id) {
                origin.mark_busy_complete();
            }
        }

        let all_passed = pending
            .received
            .iter()
            .all(|v| matches!(v.verdict, Verdict::Pass));

        if all_passed {
            tracing::info!(origin = %origin_id, "all judges passed");

            let system_entry = ChatEntry::system("✓ All judges passed evaluation.");
            let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: origin_id.clone(),
                entry: system_entry,
            }));
        } else {
            // Build consolidated failure summary.
            let failures: Vec<&ReceivedVerdict> = pending
                .received
                .iter()
                .filter(|v| matches!(v.verdict, Verdict::Fail(..)))
                .collect();

            let mut summary = String::from("Judge evaluation failed:\n\n");
            for verdict in &failures {
                if let Verdict::Fail(ref reason) = verdict.verdict {
                    let _ = write!(summary, "### {} (failed)\n{reason}\n\n", verdict.judge_name);
                }
            }

            tracing::info!(
                origin = %origin_id,
                failed_count = failures.len(),
                "some judges failed, dispatching consolidated summary"
            );

            let user_entry = ChatEntry::user(summary);
            let _ = ctx.send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
                session_id: origin_id.clone(),
                entry: user_entry,
            }));
        }
    }

    /// Handle a request to cancel a pending judge evaluation.
    ///
    /// Clears pending state, cancels all remaining judge sessions,
    /// clears busy on the origin, and pushes a system message.
    fn handle_cancel_pending(
        &mut self,
        payload: &super::protocol::CancelPendingJudgeEvaluation,
        ctx: &ActorContext,
    ) {
        let origin_id = &payload.origin_session_id;

        let Some(pending) = self.pending.remove(origin_id) else {
            return;
        };

        // Cancel all remaining judge sessions.
        for judge_session_id in pending.judges.keys() {
            let _ = ctx.send_command(Command::CancelStream(
                crate::feat::provider::protocol::command::CancelStream {
                    session_id: judge_session_id.clone(),
                },
            ));
        }

        // Clear busy on origin.
        {
            let mut guard = self.state.write();
            if let Some(origin) = guard.session.get_mut(origin_id) {
                origin.mark_busy_complete();
            }
        }

        // Push system message.
        let notification = ChatEntry::system("Judge evaluation cancelled.");
        let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
            session_id: origin_id.clone(),
            entry: notification,
        }));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use std::sync::Arc;

    use crate::common::actor::{Actor, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::judge::{JudgeCoordinatorActorDeps, JudgeMeta};
    use crate::feat::session::chat_session::{ChatSessionState, SessionPhase};
    use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
    use crate::protocol::{Command, Event, SessionId};

    use super::*;

    fn create_actor(state: State) -> (JudgeCoordinatorActor, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new(
            "judge-coordinator-test",
            sink.clone() as Arc<dyn MessageSink>,
        );
        let deps = JudgeCoordinatorActorDeps { state };
        let actor = JudgeCoordinatorActor::activate(deps, &mut ctx);
        (actor, sink, ctx)
    }

    fn make_origin_session() -> (SessionId, ChatSessionState) {
        let session = ChatSessionState::new();
        let id = session.session_id().clone();
        (id, session)
    }

    fn make_judge_session(origin_id: SessionId, attached: bool) -> (SessionId, ChatSessionState) {
        let mut session = ChatSessionState::new();
        let id = session.session_id().clone();
        session.set_judge(JudgeMeta {
            origin_session: origin_id,
            is_attached: attached,
            judge_name: "test-judge".to_string(),
            auto_reset: None,
        });
        (id, session)
    }

    fn idle_event(session_id: SessionId) -> ActorEnvelope<NoDirectMsg> {
        ActorEnvelope::Event(Event::SessionPhaseChanged(SessionPhaseChanged {
            session_id,
            old_phase: SessionPhase::Sending,
            new_phase: SessionPhase::Idle,
        }))
    }

    fn verdict_event(
        judge_session_id: SessionId,
        origin_session_id: SessionId,
        verdict: Verdict,
    ) -> ActorEnvelope<NoDirectMsg> {
        ActorEnvelope::Event(Event::JudgeVerdict(JudgeVerdict {
            judge_session_id,
            origin_session_id,
            judge_name: "test-judge".to_string(),
            verdict,
        }))
    }

    fn find_commands(commands: &[Command], predicate: impl Fn(&Command) -> bool) -> Vec<&Command> {
        commands.iter().filter(|c| predicate(c)).collect()
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn no_attached_judges_emits_no_commands() {
        // Given an origin session with no attached judges.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle.
        actor.handle(idle_event(origin_id), &ctx).await;

        // Then no commands are emitted.
        let commands = sink.commands();
        assert!(commands.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn attached_judge_triggered_on_origin_idle() {
        // Given an origin session with one attached judge.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle.
        actor.handle(idle_event(origin_id), &ctx).await;

        // Then an EnqueueUserMessage is sent to the judge session.
        let commands = sink.commands();
        let enqueue_commands = find_commands(
            &commands,
            |c| matches!(c, Command::EnqueueUserMessage(msg) if msg.session_id == judge_id),
        );
        assert_eq!(enqueue_commands.len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn detached_judge_not_triggered() {
        // Given an origin session with a detached judge.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (_judge_id, judge) = make_judge_session(origin_id.clone(), false);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle.
        actor.handle(idle_event(origin_id), &ctx).await;

        // Then no commands are emitted.
        let commands = sink.commands();
        assert!(commands.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn judge_idle_does_not_trigger_self() {
        // Given a judge session that transitions to Idle.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state);

        // When the JUDGE session goes Idle (not the origin).
        actor
            .handle(
                ActorEnvelope::Event(Event::SessionPhaseChanged(SessionPhaseChanged {
                    session_id: judge_id,
                    old_phase: SessionPhase::Sending,
                    new_phase: SessionPhase::Idle,
                })),
                &ctx,
            )
            .await;

        // Then no commands are emitted (judge sessions are not origins).
        let commands = sink.commands();
        assert!(commands.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn single_judge_pass_dispatches_system_message() {
        // Given an origin with an attached judge.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle and judge passes.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();
        actor
            .handle(
                verdict_event(judge_id, origin_id.clone(), Verdict::Pass),
                &ctx,
            )
            .await;

        // Then a PushChatEntry system message is sent to the origin.
        let commands = sink.commands();
        let push_commands = find_commands(
            &commands,
            |c| matches!(c, Command::PushChatEntry(msg) if msg.session_id == origin_id),
        );
        assert_eq!(push_commands.len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn single_judge_fail_dispatches_user_message() {
        // Given an origin with an attached judge.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle and judge fails.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();
        actor
            .handle(
                verdict_event(
                    judge_id,
                    origin_id.clone(),
                    Verdict::Fail("missing tests".into()),
                ),
                &ctx,
            )
            .await;

        // Then an EnqueueUserMessage is sent to the origin with the failure summary.
        let commands = sink.commands();
        let enqueue_commands = find_commands(
            &commands,
            |c| matches!(c, Command::EnqueueUserMessage(msg) if msg.session_id == origin_id),
        );
        assert_eq!(enqueue_commands.len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn multiple_judges_mixed_verdicts_dispatches_failure_summary() {
        // Given an origin with three attached judges.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);

        let mut judge_sessions = Vec::new();
        for i in 0..3 {
            let mut session = ChatSessionState::new();
            let id = session.session_id().clone();
            session.set_judge(JudgeMeta {
                origin_session: origin_id.clone(),
                is_attached: true,
                judge_name: format!("j-{i}"),
                auto_reset: None,
            });
            state.write().session.insert(session);
            judge_sessions.push(id);
        }

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();

        // Judges 0 and 2 pass, judge 1 fails.
        actor
            .handle(
                verdict_event(judge_sessions[0].clone(), origin_id.clone(), Verdict::Pass),
                &ctx,
            )
            .await;
        actor
            .handle(
                verdict_event(
                    judge_sessions[1].clone(),
                    origin_id.clone(),
                    Verdict::Fail("bad formatting".into()),
                ),
                &ctx,
            )
            .await;
        // Only 2 of 3 — not yet complete.
        assert!(sink.commands().is_empty());

        // Judge 2 passes — all verdicts in.
        actor
            .handle(
                verdict_event(judge_sessions[2].clone(), origin_id.clone(), Verdict::Pass),
                &ctx,
            )
            .await;

        // Then an EnqueueUserMessage is sent with consolidated failure summary.
        let commands = sink.commands();
        let enqueue_commands = find_commands(
            &commands,
            |c| matches!(c, Command::EnqueueUserMessage(msg) if msg.session_id == origin_id),
        );
        assert_eq!(enqueue_commands.len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn all_judges_pass_dispatches_system_message() {
        // Given an origin with two attached judges.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);

        let mut judge_sessions = Vec::new();
        for i in 0..2 {
            let mut session = ChatSessionState::new();
            let id = session.session_id().clone();
            session.set_judge(JudgeMeta {
                origin_session: origin_id.clone(),
                is_attached: true,
                judge_name: format!("j-{i}"),
                auto_reset: None,
            });
            state.write().session.insert(session);
            judge_sessions.push(id);
        }

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle and both judges pass.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();
        actor
            .handle(
                verdict_event(judge_sessions[0].clone(), origin_id.clone(), Verdict::Pass),
                &ctx,
            )
            .await;
        actor
            .handle(
                verdict_event(judge_sessions[1].clone(), origin_id.clone(), Verdict::Pass),
                &ctx,
            )
            .await;

        // Then a PushChatEntry system message is sent to the origin.
        let commands = sink.commands();
        let push_commands = find_commands(
            &commands,
            |c| matches!(c, Command::PushChatEntry(msg) if msg.session_id == origin_id),
        );
        assert_eq!(push_commands.len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stale_verdict_discarded_when_origin_leaves_idle() {
        // Given an origin with an attached judge.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state.clone());

        // When origin goes Idle and trigger is sent.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();

        // Origin starts a new turn (leaves Idle).
        state
            .write()
            .session
            .get_mut(&origin_id)
            .expect("origin exists")
            .core
            .ephemeral
            .phase = SessionPhase::Sending;

        // Judge verdict arrives — stale.
        actor
            .handle(
                verdict_event(judge_id, origin_id.clone(), Verdict::Pass),
                &ctx,
            )
            .await;

        // Then no commands are emitted (stale verdict discarded).
        let commands = sink.commands();
        assert!(commands.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn verdict_for_unknown_origin_ignored() {
        // Given a coordinator with no pending entries.
        let state = State::new(AppState::default());
        let (mut actor, sink, ctx) = create_actor(state);

        // When a verdict arrives for an origin with no pending entry.
        let judge_id = SessionId::new();
        let origin_id = SessionId::new();
        actor
            .handle(verdict_event(judge_id, origin_id, Verdict::Pass), &ctx)
            .await;

        // Then no commands are emitted.
        let commands = sink.commands();
        assert!(commands.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn origin_session_gone_discards_pending() {
        // Given an origin with an attached judge.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state.clone());

        // When origin goes Idle.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();

        // Origin session is removed.
        state.write().session.remove(&origin_id);

        // Judge verdict arrives — origin gone.
        actor
            .handle(verdict_event(judge_id, origin_id, Verdict::Pass), &ctx)
            .await;

        // Then no commands are emitted.
        let commands = sink.commands();
        assert!(commands.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn non_idle_phase_ignored() {
        // Given an origin session.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (_judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin transitions to Streaming (not Idle).
        actor
            .handle(
                ActorEnvelope::Event(Event::SessionPhaseChanged(SessionPhaseChanged {
                    session_id: origin_id,
                    old_phase: SessionPhase::Sending,
                    new_phase: SessionPhase::Streaming,
                })),
                &ctx,
            )
            .await;

        // Then no commands are emitted.
        let commands = sink.commands();
        assert!(commands.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn system_notification_pushed_to_origin_on_trigger() {
        // Given an origin session with two attached judges.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);

        for name in ["counter", "accuracy"] {
            let mut session = ChatSessionState::new();
            session.set_judge(JudgeMeta {
                origin_session: origin_id.clone(),
                is_attached: true,
                judge_name: name.to_string(),
                auto_reset: None,
            });
            state.write().session.insert(session);
        }

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;

        // Then a PushChatEntry with a system notification is sent to the origin.
        let commands = sink.commands();
        let push_commands: Vec<_> = commands
            .iter()
            .filter_map(|c| match c {
                Command::PushChatEntry(msg) if msg.session_id == origin_id => Some(msg),
                _ => None,
            })
            .collect();
        assert_eq!(push_commands.len(), 1);
        let notification_text = push_commands[0].entry.text();
        assert!(
            notification_text.contains("counter"),
            "notification should list 'counter' judge"
        );
        assert!(
            notification_text.contains("accuracy"),
            "notification should list 'accuracy' judge"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn origin_marked_busy_on_trigger() {
        // Given an origin session with an attached judge.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (_judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, _sink, ctx) = create_actor(state.clone());

        // When origin goes Idle.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;

        // Then the origin session is marked busy.
        let guard = state.read();
        let origin = guard.session.get(&origin_id).expect("origin exists");
        assert!(
            origin.is_busy(),
            "origin should be marked busy after triggering judges"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn origin_busy_cleared_on_consolidation() {
        // Given an origin with an attached judge.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state.clone());

        // When origin goes Idle and judge passes.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();
        actor
            .handle(
                verdict_event(judge_id, origin_id.clone(), Verdict::Pass),
                &ctx,
            )
            .await;

        // Then the origin session is no longer busy.
        let guard = state.read();
        let origin = guard.session.get(&origin_id).expect("origin exists");
        assert!(
            !origin.is_busy(),
            "origin should not be busy after consolidation"
        );
    }

    // --- Phase 2: Judge idle retry tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn judge_idle_without_verdict_retries_evaluation() {
        // Given an origin with an attached judge, origin went Idle (triggering the judge).
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle (triggers judge).
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();

        // When the judge goes Idle WITHOUT producing a verdict.
        actor.handle(idle_event(judge_id.clone()), &ctx).await;

        // Then a new EnqueueUserMessage (retry trigger) is sent to the judge.
        let commands = sink.commands();
        let retry_triggers = find_commands(
            &commands,
            |c| matches!(c, Command::EnqueueUserMessage(msg) if msg.session_id == judge_id),
        );
        assert_eq!(
            retry_triggers.len(),
            1,
            "judge idle without verdict should trigger exactly one retry"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn judge_idle_after_verdict_no_retry() {
        // Given an origin with an attached judge, judge already submitted a verdict.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state);

        // When origin goes Idle and judge submits a Pass verdict.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        actor
            .handle(verdict_event(judge_id.clone(), origin_id.clone(), Verdict::Pass), &ctx)
            .await;
        sink.clear();

        // When the judge goes Idle again (e.g., from a delayed phase change).
        actor.handle(idle_event(judge_id.clone()), &ctx).await;

        // Then no retry trigger is emitted — verdict was already received.
        let commands = sink.commands();
        let retry_triggers = find_commands(
            &commands,
            |c| matches!(c, Command::EnqueueUserMessage(msg) if msg.session_id == judge_id),
        );
        assert!(
            retry_triggers.is_empty(),
            "judge idle after verdict should not trigger retry"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn judge_retry_exhausted_force_cancels_origin() {
        // Given an origin with an attached judge.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);
        let (judge_id, judge) = make_judge_session(origin_id.clone(), true);
        state.write().session.insert(judge);

        let (mut actor, sink, ctx) = create_actor(state.clone());

        // When origin goes Idle (trigger count = 0).
        actor.handle(idle_event(origin_id.clone()), &ctx).await;

        // When judge goes Idle 3 times (retry 1, retry 2, then exhausted at retry 3).
        // retry_count increments before check: 1 < 3 = retry, 2 < 3 = retry, 3 >= 3 = exhaust.
        for _ in 0..3 {
            actor.handle(idle_event(judge_id.clone()), &ctx).await;
        }

        // Then the origin busy is cleared.
        let guard = state.read();
        let origin_session = guard.session.get(&origin_id).expect("origin exists");
        assert!(
            !origin_session.is_busy(),
            "origin should not be busy after judge retries exhausted"
        );
        drop(guard);

        // And a failure summary was dispatched to the origin.
        let commands = sink.commands();
        let failure_msgs = find_commands(
            &commands,
            |c| matches!(c, Command::EnqueueUserMessage(msg) if msg.session_id == origin_id),
        );
        assert_eq!(
            failure_msgs.len(),
            1,
            "should dispatch one failure summary to origin"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn partial_judge_exhaust_waits_for_remaining() {
        // Given an origin with two attached judges.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);

        let mut judge_sessions = Vec::new();
        for i in 0..2 {
            let mut session = ChatSessionState::new();
            let id = session.session_id().clone();
            session.set_judge(JudgeMeta {
                origin_session: origin_id.clone(),
                is_attached: true,
                judge_name: format!("j-{i}"),
                auto_reset: None,
            });
            state.write().session.insert(session);
            judge_sessions.push(id);
        }

        let (mut actor, sink, ctx) = create_actor(state.clone());

        // When origin goes Idle.
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();

        // When judge 0 exhausts all retries (3 idle cycles).
        for _ in 0..3 {
            actor.handle(idle_event(judge_sessions[0].clone()), &ctx).await;
        }

        // Then origin is still busy (judge 1 is still pending).
        let guard = state.read();
        let origin_session = guard.session.get(&origin_id).expect("origin exists");
        assert!(
            origin_session.is_busy(),
            "origin should still be busy while other judge is pending"
        );
        drop(guard);

        // And no consolidation message yet.
        let commands = sink.commands();
        let failure_msgs = find_commands(
            &commands,
            |c| matches!(c, Command::EnqueueUserMessage(msg) if msg.session_id == origin_id),
        );
        assert!(
            failure_msgs.is_empty(),
            "should not consolidate until all judges report"
        );

        // When judge 1 finally delivers a verdict.
        actor
            .handle(verdict_event(judge_sessions[1].clone(), origin_id.clone(), Verdict::Pass), &ctx)
            .await;

        // Then consolidation happens and origin is no longer busy.
        let guard = state.read();
        let origin_session = guard.session.get(&origin_id).expect("origin exists");
        assert!(
            !origin_session.is_busy(),
            "origin should not be busy after all judges report"
        );
    }

    fn make_judge_meta(auto_reset: Option<bool>) -> JudgeMeta {
        JudgeMeta {
            origin_session: SessionId::new(),
            is_attached: true,
            judge_name: "test-judge".to_string(),
            auto_reset,
        }
    }

    fn make_judge_definition(name: &str, auto_reset: bool) -> super::super::Judge {
        super::super::Judge {
            name: name.to_string(),
            description: format!("{name} judge"),
            body: format!("{name} body"),
            model: None,
            auto_reset,
            file_path: std::path::PathBuf::from(format!("/judges/{name}.md")),
        }
    }

    #[rstest::rstest]
    fn resolve_auto_reset_with_override_true() {
        // Given a meta with auto_reset = Some(true).
        let meta = make_judge_meta(Some(true));
        let judges = vec![make_judge_definition("test-judge", false)];

        // When resolving.
        let result = resolve_effective_auto_reset(&meta, &judges);

        // Then the override takes precedence (true, even though judge default is false).
        assert!(result);
    }

    #[rstest::rstest]
    fn resolve_auto_reset_with_override_false() {
        // Given a meta with auto_reset = Some(false).
        let meta = make_judge_meta(Some(false));
        let judges = vec![make_judge_definition("test-judge", true)];

        // When resolving.
        let result = resolve_effective_auto_reset(&meta, &judges);

        // Then the override takes precedence (false, even though judge default is true).
        assert!(!result);
    }

    #[rstest::rstest]
    fn resolve_auto_reset_falls_back_to_judge_true() {
        // Given a meta with auto_reset = None and a judge with auto_reset = true.
        let meta = make_judge_meta(None);
        let judges = vec![make_judge_definition("test-judge", true)];

        // When resolving.
        let result = resolve_effective_auto_reset(&meta, &judges);

        // Then it falls back to the judge's default (true).
        assert!(result);
    }

    #[rstest::rstest]
    fn resolve_auto_reset_falls_back_to_judge_false() {
        // Given a meta with auto_reset = None and a judge with auto_reset = false.
        let meta = make_judge_meta(None);
        let judges = vec![make_judge_definition("test-judge", false)];

        // When resolving.
        let result = resolve_effective_auto_reset(&meta, &judges);

        // Then it falls back to the judge's default (false).
        assert!(!result);
    }

    #[rstest::rstest]
    fn resolve_auto_reset_unknown_judge_returns_false() {
        // Given a meta with auto_reset = None and no matching judge.
        let meta = make_judge_meta(None);
        let judges = vec![make_judge_definition("other-judge", true)];

        // When resolving.
        let result = resolve_effective_auto_reset(&meta, &judges);

        // Then it returns false (judge not found).
        assert!(!result);
    }

    // --- Phase 3: Cancel pending judge evaluation tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn coordinator_handles_cancel_pending() {
        // Given an origin with two attached judges, both pending.
        let state = State::new(AppState::default());
        let (origin_id, origin) = make_origin_session();
        state.write().session.insert(origin);

        let mut judge_ids = Vec::new();
        for i in 0..2 {
            let mut session = ChatSessionState::new();
            let id = session.session_id().clone();
            session.set_judge(JudgeMeta {
                origin_session: origin_id.clone(),
                is_attached: true,
                judge_name: format!("j-{i}"),
                auto_reset: None,
            });
            state.write().session.insert(session);
            judge_ids.push(id);
        }

        let (mut actor, sink, ctx) = create_actor(state.clone());

        // When origin goes Idle (triggers judges).
        actor.handle(idle_event(origin_id.clone()), &ctx).await;
        sink.clear();

        // Verify origin is busy.
        {
            let guard = state.read();
            let origin_session = guard.session.get(&origin_id).expect("origin exists");
            assert!(origin_session.is_busy(), "origin should be busy before cancel");
        }

        // When cancelling the pending evaluation.
        let cancel_cmd = Command::CancelPendingJudgeEvaluation(
            super::super::protocol::CancelPendingJudgeEvaluation {
                origin_session_id: origin_id.clone(),
            },
        );
        actor
            .handle(ActorEnvelope::Command(cancel_cmd), &ctx)
            .await;

        // Then the origin is no longer busy.
        let guard = state.read();
        let origin_session = guard.session.get(&origin_id).expect("origin exists");
        assert!(
            !origin_session.is_busy(),
            "origin should not be busy after cancel"
        );
        drop(guard);

        // And CancelStream is sent to both judges.
        let commands = sink.commands();
        let cancel_streams: Vec<_> = commands
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Command::CancelStream(crate::feat::provider::protocol::command::CancelStream { session_id })
                    if judge_ids.contains(session_id)
                )
            })
            .collect();
        assert_eq!(
            cancel_streams.len(),
            2,
            "should cancel both judge sessions"
        );

        // And a system message is pushed to the origin.
        let push_cmds: Vec<_> = commands
            .iter()
            .filter_map(|c| match c {
                Command::PushChatEntry(msg) if msg.session_id == origin_id => Some(msg),
                _ => None,
            })
            .collect();
        assert_eq!(push_cmds.len(), 1);
        assert!(push_cmds[0].entry.text().contains("cancelled"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn coordinator_cancel_pending_ignores_unknown_origin() {
        // Given a coordinator with no pending entries.
        let state = State::new(AppState::default());
        let (mut actor, sink, ctx) = create_actor(state);

        // When cancelling for an unknown origin.
        let cancel_cmd = Command::CancelPendingJudgeEvaluation(
            super::super::protocol::CancelPendingJudgeEvaluation {
                origin_session_id: SessionId::new(),
            },
        );
        actor
            .handle(ActorEnvelope::Command(cancel_cmd), &ctx)
            .await;

        // Then no commands are emitted.
        let commands = sink.commands();
        assert!(commands.is_empty(), "should emit nothing for unknown origin");
    }
}
