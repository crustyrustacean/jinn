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

//! Stall watchdog actor.
//!
//! Periodically scans every session in `Sending` or `Streaming` and checks
//! how long ago its chat history was last mutated (`last_history_activity_at`).
//! If that window exceeds `history_stall_timeout_secs`, the turn is treated
//! as hung — exactly like a hard provider error — and the session is retried.
//!
//! "History activity" is the only signal that counts: a new entry pushed
//! (`push_entry`), assistant text growing (`append_stream_token`), or thinking
//! text growing (`append_thinking_token`). Provider keepalives that produce
//! no `StreamEvent`, a half-open connection emitting nothing, an unanswered
//! tool batch — all of these leave the history untouched and are therefore
//! detected here.
//!
//! When a stall is detected:
//! - if the session's stall-retry budget is not exhausted, publish
//!   [`RetryStalledSession`](crate::feat::session::protocol::retry_stalled_session::RetryStalledSession);
//! - otherwise publish
//!   [`CancelStream`](crate::feat::provider::protocol::command::CancelStream).
//!
//! The per-session attempt counter resets the moment history activity resumes
//! (the timestamp advances between ticks), so a session that recovers then
//! stalls again gets a fresh budget.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use jiff::Timestamp;
use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::state::State;
use crate::feat::provider::protocol::command::CancelStream;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::retry_stalled_session::RetryStalledSession;
use crate::protocol::SessionId;

/// How often the watchdog scans for stalled sessions.
///
/// Picked short relative to the (default 300s) timeout so detection latency
/// is low, but not so frequent that it hammers the read lock.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Internal tick message driving each scan.
struct Tick;

/// Actor that detects hung sessions and retries (or cancels) them.
pub struct StallWatchdogActor {
    deps: ActorDeps,
    state: State,
    /// Per-session stall-retry attempt count, tracking how many retries are
    /// in flight since the last observed history activity.
    attempts: HashMap<SessionId, u32>,
    /// Timestamp of the most recent `RetryStalledSession` publish per session.
    /// Used to enforce exponential backoff between consecutive retries: a
    /// stalled session is not retried again until `compute_stall_backoff`
    /// has elapsed since its last retry.
    last_retry_at: HashMap<SessionId, Timestamp>,
    /// Sessions currently observed in an active turn (Sending/Streaming).
    /// Used to detect turn boundaries: when a session leaves the active phases,
    /// its retry budget is reset (the turn completed or was canceled).
    tracked: HashSet<SessionId>,
}

/// Dependencies for spawning a [`StallWatchdogActor`].
#[derive(Clone)]
pub struct StallWatchdogActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
    /// Shared application state.
    pub state: State,
}

impl Actor for StallWatchdogActor {
    type Args = StallWatchdogActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        // Drive the scan loop via self-addressed Tick messages. The task dies
        // with the actor because it holds a clone of `actor_ref`.
        let timer_ref = actor_ref.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(POLL_INTERVAL);
            // The first tick fires immediately; skip it.
            interval.tick().await;
            loop {
                interval.tick().await;
                let _ = timer_ref.tell(Tick).await;
            }
        });

        Ok(Self {
            deps: args.deps,
            state: args.state,
            attempts: HashMap::new(),
            last_retry_at: HashMap::new(),
            tracked: HashSet::new(),
        })
    }
}

impl Message<Tick> for StallWatchdogActor {
    type Reply = ();

    async fn handle(&mut self, _msg: Tick, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        self.scan().await;
    }
}

impl BusPublish for StallWatchdogActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        self.deps.bus()
    }
}

impl StallWatchdogActor {
    /// Exponential backoff with full jitter (AWS style), mirroring the shape of
    /// `jinn_provider::retry::RetryingLlmService::compute_delay`: returns
    /// `base_delay * 2^attempt` capped at `max_delay`, then jittered down to
    /// `[0, capped]` so concurrent stalls don’t thunder.
    #[must_use]
    fn compute_stall_backoff(base_delay: Duration, max_delay: Duration, attempt: u32) -> Duration {
        let base_secs = base_delay.as_secs_f64();
        let exponential = base_secs * 2_f64.powi(i32::try_from(attempt).unwrap_or(i32::MAX));
        let capped = exponential.min(max_delay.as_secs_f64());
        if capped <= 0.0 {
            return Duration::ZERO;
        }
        let final_delay = rand::random_range(0.0..capped);
        let millis = (final_delay * 1000.0) as u64;
        Duration::from_millis(millis)
    }
    /// One scan pass: inspect every active-turn session and act on stalls.
    async fn scan(&mut self) {
        let prefs = self.deps.services.user_preferences_storage.read();
        let timeout_secs = prefs.history_stall_timeout_secs;
        let max_retries = prefs.stall_retry_max_retries;
        let base_delay = Duration::from_secs(prefs.stall_retry_base_delay_secs);
        let max_delay = Duration::from_secs(prefs.stall_retry_max_delay_secs);
        drop(prefs);

        let now = Timestamp::now();

        // First pass (read lock): decide which active-turn sessions are stalled.
        // A session's stall-retry counter is reset only at a turn boundary —
        // when it leaves the active phases (reaches `Idle`), meaning the turn
        // genuinely completed. Resetting on timestamp jitter would defeat the
        // budget: every retry re-seeds `last_history_activity_at`, which would
        // otherwise look like recovery and grant infinite retries to a
        // perpetually hung provider.
        let mut stalled: Vec<(SessionId, bool)> = Vec::new();
        let mut active_now: Vec<SessionId> = Vec::new();
        {
            let guard = self.state.read();
            for (id, session) in guard.session.iter() {
                let phase = session.phase();
                if !matches!(phase, PhaseKind::Sending | PhaseKind::Streaming) {
                    continue;
                }
                active_now.push(id.clone());

                let last = session.core.last_history_activity_at;
                let elapsed_secs = match now.since(last) {
                    Ok(span) => span.get_seconds(),
                    Err(_) => continue,
                };

                if elapsed_secs >= timeout_secs as i64 {
                    let attempts = self.attempts.get(id).copied().unwrap_or(0);
                    stalled.push((id.clone(), attempts < max_retries));
                }
            }
        }

        // Turn-boundary reset: any previously-tracked session no longer in an
        // active phase has completed (or been canceled) — clear its budget so
        // the next turn starts fresh.
        let finished: Vec<SessionId> = self
            .tracked
            .iter()
            .filter(|id| !active_now.contains(id))
            .cloned()
            .collect();
        for id in &finished {
            self.attempts.remove(id);
            self.last_retry_at.remove(id);
        }
        self.tracked.retain(|id| active_now.contains(id));
        // Newly active sessions (a fresh turn) start with a clean budget.
        for id in &active_now {
            if !self.tracked.contains(id) {
                self.attempts.remove(id);
                self.last_retry_at.remove(id);
                self.tracked.insert(id.clone());
                self.tracked.insert(id.clone());
            }
        }

        for (session_id, under_budget) in stalled {
            if under_budget {
                // Exponential-backoff gate: if we already retried this session,
                // wait at least `compute_stall_backoff(attempt)` before trying
                // again, so a misbehaving provider that answers-and-stalls
                // rapidly is not hammered. Mirrors RetryingLlmService::compute_delay.
                let attempt = self.attempts.get(&session_id).copied().unwrap_or(0);
                if let Some(last) = self.last_retry_at.get(&session_id) {
                    let required = Self::compute_stall_backoff(base_delay, max_delay, attempt);
                    let elapsed = match now.since(*last) {
                        Ok(span) => Duration::from_secs(span.get_seconds().max(0).unsigned_abs()),
                        Err(_) => Duration::ZERO,
                    };
                    if elapsed < required {
                        tracing::trace!(
                            session_id = %session_id,
                            ?required,
                            ?elapsed,
                            "stalled but within backoff window — deferring retry"
                        );
                        continue;
                    }
                }

                let attempts = self.attempts.entry(session_id.clone()).or_insert(0);
                *attempts += 1;
                self.last_retry_at.insert(session_id.clone(), now);
                tracing::warn!(
                    session_id = %session_id,
                    attempt = *attempts,
                    max_retries,
                    "session stalled — retrying turn"
                );
                self.publish(RetryStalledSession { session_id }).await;
            } else {
                tracing::warn!(
                    session_id = %session_id,
                    max_retries,
                    "session stalled past retry budget — canceling"
                );
                self.attempts.remove(&session_id);
                self.last_retry_at.remove(&session_id);
                self.publish(CancelStream { session_id }).await;
            }
        }
    }

    /// Test-only constructor: builds the actor state without spawning the
    /// tick loop. Used by unit tests that drive `scan()` explicitly.
    #[cfg(test)]
    pub(crate) fn for_test(deps: ActorDeps, state: State) -> Self {
        Self {
            deps,
            state,
            attempts: HashMap::new(),
            last_retry_at: HashMap::new(),
            tracked: HashSet::new(),
        }
    }

    /// Test-only entry point that runs a single scan synchronously.
    #[cfg(test)]
    pub(crate) async fn scan_once(&mut self) {
        self.scan().await;
    }

    /// Test-only: backdate a session's `last_retry_at` so the backoff gate
    /// observes an elapsed window larger than the computed delay.
    #[cfg(test)]
    pub(crate) fn backdate_last_retry(&mut self, session_id: &SessionId, ago: Duration) {
        let ts = Timestamp::now()
            .checked_sub(ago)
            .unwrap_or_else(|_| Timestamp::now());
        self.last_retry_at.insert(session_id.clone(), ts);
    }
}
#[cfg(test)]
mod tests;
