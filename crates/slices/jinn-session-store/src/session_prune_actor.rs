//! Background actor that prunes stale session trees and shrinks the database.
//!
//! A trouper [`ServiceActor`] driven by a self-addressed tick, following the
//! queue-and-batch model of the search-index actor (trouper has no
//! scheduler): the first tick self-schedules [`INITIAL_DELAY`] after spawn,
//! and each tick's **last** act is to schedule the next one. When the queue
//! runs dry the tick refills it from a fresh selection; while sessions
//! remain queued each tick deletes at most [`PRUNE_BATCH`] of them in one
//! short transaction and hands the freed pages back with a bounded
//! incremental vacuum, rescheduling [`DRAIN_INTERVAL`] later. Once drained
//! (or retention disabled) the next sweep follows [`SWEEP_INTERVAL`] later
//! — ticks are therefore spaced from completion, not per wall-clock hour.
//!
//! Chunking is what keeps shutdown responsive: graceful shutdown waits for
//! the in-flight tick to finish, so every tick must be short. One small
//! transaction per tick means a drain in progress delays shutdown by at
//! most that transaction — well inside the deadline — instead of holding
//! the database write lock for one giant delete of every stale tree at
//! once. Crash-safety is inherited, not built: the selection
//! ([`prune_candidates`]) is idempotent, so a crash mid-drain simply
//! re-finds the remainder on the next sweep.
//!
//! Owns no `AppState` fields, no cell, no routes — its writes go to the
//! store (`delete_many`, `incremental_vacuum`) and to the `session-prune`
//! dashboard row (`jinn_slices::ServiceStatusUpdate` publishes, the
//! indexer's row precedent). On each tick it reads the `[maintenance]`
//! section of `jinn.toml` — the section stays named for general database
//! maintenance, under which this actor and future caretakers live; when
//! `session_retention` is absent or unparseable the sweep is a no-op
//! (jinn never deletes anything the user has not explicitly told it to),
//! the queue is dropped, and the tick still reschedules so filling the
//! section in takes effect without a restart.
//!
//! Deletion is by **session tree**: a root (or pre-orphaned child acting as
//! its own root) and every descendant qualify together, and only when
//! *every* member is archived, outside the Library, and the tree's newest
//! `updated_at` is older than the retention threshold. One fresh member
//! protects its whole family. A tree larger than the batch splits across
//! transactions — selection is a tree *rule*, not an atomicity rule; a
//! crash leaves the remainder stale and the next selection finds it. A
//! bounded incremental vacuum after every chunk hands the freed pages back
//! to the filesystem (the database is converted to incremental auto-vacuum
//! by schema migration v32).

use std::collections::HashMap;
use std::time::Duration;

use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

use jinn_core_types::SessionId;
use jinn_kernel::common::actor_deps::ActorDeps;
use jinn_preferences_config::MaintenanceConfig;
use jinn_session_list::descendant_closure;
use jinn_session_store_msg::SessionSummary;

/// How long after spawn the first sweep runs.
///
/// Long enough that the startup sweep does not compete with startup work,
/// short enough that the first prune (and its dashboard signal) appear
/// promptly.
pub const INITIAL_DELAY: Duration = Duration::from_secs(5);

/// How long after the queue drains the next sweep refills it.
///
/// The idle cadence, measured from the previous tick's completion.
pub const SWEEP_INTERVAL: Duration = Duration::from_hours(1);

/// How long after a drain chunk the next one runs while sessions remain
/// queued.
pub const DRAIN_INTERVAL: Duration = Duration::from_secs(2);

/// How many sessions one tick's delete transaction may cover.
///
/// Sessions are big (a transcript with full tool outputs), so a large chunk
/// would still hold the SQLite write lock — and with it graceful shutdown —
/// for a long time. This keeps every tick sub-second on slow storage
/// (spinning-disk ZFS mirrors included) while a big first prune drains
/// across ticks: one session per transaction.
pub const PRUNE_BATCH: usize = 1;

/// The most pages one incremental vacuum pass may hand back to the OS.
///
/// Bounded so a sweep never holds the database in a long vacuum: a few
/// hundred 4 KiB pages per hour is plenty for normal pruning, and an
/// oversized backlog (e.g. the first sweep after enabling retention on an
/// old database) drains across sweeps, since `incremental_vacuum` returns
/// as soon as the freelist is empty anyway.
pub const MAX_VACUUM_PAGES: u32 = 512;

/// The session prune actor's static trouper path.
pub const SESSION_PRUNE_PATH: &str = "session-prune";

/// Dashboard row this actor publishes prune progress under. Must equal
/// [`SESSION_PRUNE_PATH`] — the runtime announces the actor's lifecycle on
/// that name, and a mismatch would split the announcement from the status
/// publishes (the indexer precedent).
pub const SESSION_PRUNE_ROW_NAME: &str = "session-prune";

/// Dependencies for [`SessionPruneActor`].
#[derive(Clone)]
pub struct SessionPruneActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// Delay before the first sweep. Production uses [`INITIAL_DELAY`];
    /// tests inject a small value so convergence assertions don't wait on
    /// the default.
    pub initial_delay: Duration,
    /// Interval between sweeps once the queue is drained, measured from the
    /// previous tick's completion. Production uses [`SWEEP_INTERVAL`];
    /// tests inject milliseconds.
    pub sweep_interval: Duration,
    /// Interval between drain ticks while sessions remain queued.
    /// Production uses [`DRAIN_INTERVAL`]; tests inject milliseconds.
    pub drain_interval: Duration,
    /// How many sessions one tick's delete transaction may cover.
    /// Production uses [`PRUNE_BATCH`]; tests inject `1` (one session per
    /// tick) or a large value to exercise the batching contract.
    pub batch: usize,
}

/// The session prune actor: refill, delete one chunk, vacuum, publish,
/// reschedule.
pub struct SessionPruneActor {
    deps: ActorDeps,
    sweep_interval: Duration,
    drain_interval: Duration,
    batch: usize,
    /// Sessions selected for deletion, drained front-first one chunk per
    /// tick. A `Vec`, not a set: selection order is stable and there is no
    /// membership test worth optimizing at this batch size.
    queue: Vec<SessionId>,
}

impl ServiceActor for SessionPruneActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "trait contract: init is never called (spawn uses init_with)"
    )]
    async fn init(_args: &trouper::json::Json) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the deps via `init_with`
        // (ActorDeps carries typed handles that cannot ride JSON args).
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("SessionPruneActor is spawned via init_with"),
        )
    }
}

impl SessionPruneActor {
    /// Spawns the actor at its static trouper path and returns the path.
    ///
    /// The first tick is armed by `.kickoff` after `initial_delay` (not
    /// immediately — the sweep must not compete with startup); every later
    /// tick is scheduled by the previous one's final act.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into init_with"
    )]
    pub fn spawn(system: &ActorSystem, deps: SessionPruneActorDeps) -> ActorPath {
        let path = ActorPath::new(SESSION_PRUNE_PATH);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .init_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    Box::pin(async move {
                        Ok(Self {
                            deps: deps.deps,
                            sweep_interval: deps.sweep_interval,
                            drain_interval: deps.drain_interval,
                            batch: deps.batch,
                            queue: Vec::new(),
                        })
                    })
                }
            })
            .handles::<PruneTick>()
            .mailbox(8, trouper::inbox::OverloadPolicy::Block)
            // Arms the first sweep after the initial delay; the chain
            // self-perpetuates from there (see [`Self::reschedule`]).
            .kickoff(PruneTick, deps.initial_delay)
            .start();
        path
    }

    /// Delivers the next tick to this actor after `interval`.
    ///
    /// Called as the tick handler's **last** act — after the chunk (or
    /// no-op) completed — so intervals are measured from completion. A
    /// non-empty queue drains at the drain interval; a drained or disabled
    /// one sweeps at the sweep interval.
    fn reschedule(ctx: &mut MsgCtx<'_>, interval: Duration) {
        ctx.schedule_self(PruneTick, interval);
    }
}

/// A self-addressed prune tick: refill the queue when idle, delete one
/// chunk, vacuum, then reschedule.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, trouper::schema::Command)]
#[schema(
    description = "Session prune tick: delete one chunk of stale sessions, then incremental-vacuum."
)]
pub struct PruneTick;

impl MsgHandler<PruneTick> for SessionPruneActor {
    async fn handle(&mut self, _msg: &PruneTick, ctx: &mut MsgCtx<'_>) {
        let retention = self.retention().await;
        match retention {
            // Disabled: drop any pending deletions immediately — the user
            // said stop, so never delete — and show idle.
            None => {
                self.queue.clear();
                self.publish_idle().await;
            }
            Some(retention) => {
                // Refill only from empty: mid-drain ticks keep deleting the
                // queue they hold, so a tree selected for deletion stays
                // selected even if the store changes underneath.
                let mut readable = true;
                if self.queue.is_empty() {
                    readable = self.refresh_queue(retention).await;
                }
                // Work first, idle label only when the store read agreed
                // the queue is genuinely empty; an unreadable store
                // publishes nothing, so the previous dashboard label stays
                // up and the next tick retries.
                if !self.queue.is_empty() {
                    // Publish before the chunk so the count includes it.
                    self.publish_remaining().await;
                    self.delete_chunk().await;
                }
                if self.queue.is_empty() && readable {
                    self.publish_idle().await;
                }
            }
        }
        // The chain must stay alive even when disabled or failed: the last
        // act of every tick, without exception, is the next tick — paced by
        // whether work remains. The interval reads the queue *after* the
        // chunk, or a final chunk would leave the actor hourly-paced with
        // a non-empty queue.
        let interval = if self.queue.is_empty() {
            self.sweep_interval
        } else {
            self.drain_interval
        };
        Self::reschedule(ctx, interval);
    }
}

impl SessionPruneActor {
    /// Reads the configured session retention.
    ///
    /// `None` means deletion is disabled: the section is absent (the
    /// default), the value is empty, or the value does not parse. An
    /// unparseable value is warned about with the offending text — a
    /// retention typo may cost a log line, never a failed startup or a
    /// wrong delete.
    async fn retention(&self) -> Option<Duration> {
        let config = self.deps.services.config.get::<MaintenanceConfig>();
        let config = match config {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "failed to read the [maintenance] config section; this sweep will not delete anything"
                );
                return None;
            }
        };
        let parsed = config.retention_duration();
        if parsed.is_none()
            && let Some(raw) = config
                .session_retention
                .filter(|raw| !raw.trim().is_empty())
        {
            tracing::warn!(
                session_retention = %raw,
                "[maintenance] session_retention is not <number><m|h|d|w>; treating as disabled"
            );
        }
        std::future::ready(parsed).await
    }

    /// Refills the queue from a fresh selection of stale trees.
    ///
    /// Selection is idempotent — the same store state selects the same
    /// trees minus what is already gone — so a crash mid-drain simply
    /// re-finds the remainder here, and no durable marker table is needed.
    /// Returns whether the store was readable: on a read failure the queue
    /// stays empty and the caller publishes nothing, so the previous
    /// dashboard label stays up and the next tick retries (the indexer's
    /// refresh behavior).
    async fn refresh_queue(&mut self, retention: Duration) -> bool {
        let summaries = match self.deps.services.session_store.load_summaries().await {
            Ok(summaries) => summaries,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "prune could not read session summaries; keeping the queue empty and retrying next tick"
                );
                return false;
            }
        };

        let now = jiff::Timestamp::now();
        let stale = prune_candidates(&summaries, now, retention);
        let queued = stale.len();
        self.queue = stale;
        if queued > 0 {
            tracing::info!(
                sessions = queued,
                ?retention,
                "prune queue filled with stale sessions pending deletion"
            );
        }
        true
    }

    /// Deletes at most one batch of queued sessions, then vacuums.
    ///
    /// The chunk is one short transaction in the store ([`PRUNE_BATCH`]
    /// sessions at most), which is the whole point: graceful shutdown waits
    /// for the in-flight tick, so every tick must be short. A failed delete
    /// puts the chunk back at the front, in order, for the next tick (the
    /// indexer's failed-session requeue) — no sessions are lost, and a
    /// persistently failing store just retries each drain tick.
    async fn delete_chunk(&mut self) {
        let end = self.batch.min(self.queue.len());
        let chunk: Vec<SessionId> = self.queue.drain(..end).collect();
        if let Err(error) = self.deps.services.session_store.delete_many(&chunk).await {
            self.queue.splice(..0, chunk);
            tracing::warn!(
                error = ?error,
                "prune chunk failed to delete; the chunk is requeued for the next tick"
            );
            return;
        }
        // Freed pages go back to the filesystem within the bounded budget —
        // every chunk, not only at the end, so the drain's total lock time
        // stays as bounded as its transactions.
        if let Err(error) = self
            .deps
            .services
            .session_store
            .incremental_vacuum(MAX_VACUUM_PAGES)
            .await
        {
            tracing::warn!(
                error = ?error,
                "prune chunk deleted sessions but the incremental vacuum failed; pages return on a later chunk"
            );
        }
        tracing::info!(count = chunk.len(), "prune chunk deleted stale sessions");
    }

    /// Publishes the pending count: "N sessions pending". Called before the
    /// chunk is drained, so the count includes the chunk about to run (the
    /// indexer's publish-remaining-before-processing precedent).
    async fn publish_remaining(&self) {
        let pending = self.queue.len();
        self.publish_status(format!("{pending} sessions pending"))
            .await;
    }

    /// Publishes the idle label — the queue is genuinely drained, or
    /// retention is disabled and there is nothing this actor may delete.
    async fn publish_idle(&self) {
        self.publish_status("prune up to date".to_owned()).await;
    }

    async fn publish_status(&self, status: String) {
        self.deps
            .services
            .bus
            .publish(jinn_slices::ServiceStatusUpdate {
                name: SESSION_PRUNE_ROW_NAME.to_owned(),
                description: None,
                status_message: Some(status),
                note_tone: None,
            })
            .await;
    }
}

/// Selects every session that belongs to a prunable tree.
///
/// The store's summaries name all sessions; their `parent_session` links
/// describe trees. A tree is prunable when **every** member is archived,
/// outside the Library, and the tree's newest `updated_at` is older than
/// `now - retention` — one fresh (or library'd, or loaded) member protects
/// the whole family, because "no activity" is a statement about the
/// conversation family, not one transcript.
///
/// A session whose parent row is absent from the summaries (a pre-orphaned
/// fork) forms its own root and is judged on its own subtree alone.
#[must_use]
pub fn prune_candidates(
    summaries: &[SessionSummary],
    now: jiff::Timestamp,
    retention: Duration,
) -> Vec<SessionId> {
    let parent_of: HashMap<SessionId, Option<SessionId>> = summaries
        .iter()
        .map(|s| (s.session_id.clone(), s.parent_session.clone()))
        .collect();

    let roots: Vec<&SessionSummary> = summaries
        .iter()
        .filter(|s| match &s.parent_session {
            None => true,
            Some(parent) => !parent_of.contains_key(parent),
        })
        .collect();

    let cutoff = now - retention;
    let mut stale = Vec::new();
    for root in roots {
        let members = descendant_closure(&root.session_id, &parent_of);
        if tree_is_stale(&members, summaries, cutoff) {
            stale.extend(members);
        }
    }
    stale
}

/// Returns true when every member of one tree is archived, not in the
/// Library, and last updated before `cutoff`.
fn tree_is_stale(
    members: &[SessionId],
    summaries: &[SessionSummary],
    cutoff: jiff::Timestamp,
) -> bool {
    let by_id: HashMap<&SessionId, &SessionSummary> =
        summaries.iter().map(|s| (&s.session_id, s)).collect();
    members.iter().all(|id| {
        let Some(summary) = by_id.get(id) else {
            // A member without a summary cannot be judged; spare it.
            return false;
        };
        summary.session_state == jinn_session_store_msg::SessionState::Archived
            && !summary.in_library
            && summary.updated_at < cutoff
    })
}
