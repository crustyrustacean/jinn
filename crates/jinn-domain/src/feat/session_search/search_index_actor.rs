//! Background actor that keeps the FTS search index fresh.
//!
//! Owns no `AppState` fields — its writes go to the database (the
//! `session_fts` index table), not to shared state. Dirty sessions are
//! recorded by triggers on the `sessions` table (schema v26); this actor is
//! a message-driven state machine over an in-memory queue of dirty
//! sessions: each heartbeat refreshes the queue from the durable marker
//! table when idle, then reindexes at most one batch of sessions,
//! publishing the remaining count to the `search-index` dashboard row
//! after every session. Search results may trail the newest saves by one
//! interval; the agent's own current-turn entries are in its context
//! regardless.
//!
//! Batching replaces a time budget: one batch per heartbeat keeps the
//! handler short so the mailbox — and with it the supervised shutdown
//! handshake — stays responsive between batches, while a large backfill
//! (e.g. the first launch after the schema upgrade, hundreds of sessions)
//! drains across heartbeats. The durable `fts_dirty` table remains the
//! source of truth; the in-memory queue is only a cache, refreshed whenever
//! the heartbeat finds it empty. A session re-marked while queued is a
//! no-op (the set dedupes), and one re-marked after processing re-enters
//! the queue on the next idle refresh.

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use kameo::actor::{ActorRef, Spawn};
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
use crate::protocol::SessionId;

/// How often the actor beats in production.
pub const REINDEX_INTERVAL: Duration = Duration::from_secs(5);

/// How many sessions one heartbeat reindexes before yielding to the mailbox.
///
/// Bounds the handler's runtime without clock-watching: each heartbeat
/// processes at most one batch, so the actor stays stoppable and responsive
/// between batches, and a large backfill drains across heartbeats. Within a
/// session the store chunks the rebuild ([`REINDEX_CHUNK`] entries per
/// transaction), so one enormous session still yields between chunks.
pub const REINDEX_BATCH: usize = 10;

/// How many entries one reindex chunk transaction may index.
///
/// Small enough that a debug-build parse of a chunk (the expensive part —
/// kind JSON of full tool outputs) returns promptly and the write lock is
/// released between chunks for concurrent persists.
pub const REINDEX_CHUNK: usize = 500;

/// Dashboard row this actor publishes reindex progress under. Must match the
/// `spawn_tracked!` registration name in `actor_wiring.rs` — a mismatch would
/// silently publish into a row that doesn't exist. The row's name,
/// description, and lifecycle columns stay owned by the wiring/lifecycle
/// events; this actor only fills the status message.
pub const SEARCH_INDEX_ROW_NAME: &str = "search-index";

/// Dependencies for [`SearchIndexActor`].
#[derive(Clone)]
pub struct SearchIndexActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// Heartbeat interval. Production uses [`REINDEX_INTERVAL`]; tests
    /// inject a small value so convergence assertions don't wait on the
    /// default.
    pub interval: Duration,
    /// Sessions reindexed per heartbeat. Production uses [`REINDEX_BATCH`];
    /// tests inject `1` (one session per heartbeat) or a large value (drain
    /// everything in one heartbeat).
    pub batch: usize,
}

/// The search-index maintenance actor.
///
/// The in-memory queue is a cache of the durable `fts_dirty` table, not a
/// source of truth: a crash or a skipped heartbeat costs freshness only, and
/// the work is retried on the next heartbeat (or the next startup). The
/// queue's count drives the dashboard label between refreshes; the durable
/// table is consulted whenever the queue empties, so the row only reads
/// "index up to date" when the marker table is genuinely clean.
pub struct SearchIndexActor {
    deps: ActorDeps,
    interval: Duration,
    batch: usize,
    queue: HashSet<SessionId>,
}

impl kameo::Actor for SearchIndexActor {
    type Args = SearchIndexActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        // No drain kick here on purpose: a self-tell queued from `on_start`
        // lands in the mailbox ahead of kameo's StartupFinished signal, so
        // the supervised spawn handshake — and with it the whole actor
        // wiring — would block until the first drain completes (a
        // multi-second freeze on a large pending queue). The spawn helper
        // kicks the first heartbeat after the handshake instead.
        let _ = actor_ref; // unused without the kick; keeps the signature stable
        Ok(Self {
            deps: args.deps,
            interval: args.interval,
            batch: args.batch,
            queue: HashSet::new(),
        })
    }
}

impl BusPublish for SearchIndexActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

/// A self-addressed heartbeat: refreshes the queue when idle, reindexes one
/// batch when work is pending, then schedules the next heartbeat.
#[derive(Debug)]
pub struct ReindexTick;

impl Message<ReindexTick> for SearchIndexActor {
    type Reply = ();

    async fn handle(&mut self, _msg: ReindexTick, ctx: &mut Context<Self, Self::Reply>) {
        if self.queue.is_empty() {
            self.refresh_queue().await;
        }
        if self.queue.is_empty() {
            self.publish_up_to_date().await;
        } else {
            // Nothing is in flight yet: the whole queue is still counted in
            // self.queue.
            self.publish_remaining(0).await;
            self.process_batch().await;
        }
        self.reschedule(ctx);
    }
}

impl SearchIndexActor {
    /// Merges the store's dirty sessions into the local queue.
    ///
    /// Merging (not replacing) keeps sessions found by an earlier refresh
    /// that have since been re-marked, and makes re-marks of queued sessions
    /// no-ops. On a read failure the current queue is kept — the durable
    /// table is untouched by the actor, so the next heartbeat simply retries.
    async fn refresh_queue(&mut self) {
        match self.deps.services.session_store.dirty_session_ids().await {
            Ok(ids) => self.queue.extend(ids),
            Err(report) => {
                tracing::warn!(
                    error = ?report,
                    "failed to read the reindex queue; keeping the current queue and retrying next heartbeat"
                );
            }
        }
    }

    /// Reindexes at most one batch of queued sessions, publishing the
    /// remaining count after every session.
    ///
    /// The batch is snapshotted before processing: a session that fails is
    /// put back in the queue for a later heartbeat but is not re-processed
    /// within this one — otherwise a persistently failing session would be
    /// popped again immediately and starve the rest of the batch.
    ///
    /// Each session is reindexed chunk-wise to completion ([`REINDEX_CHUNK`]
    /// entries per transaction, resumable via the store's dirty-marker
    /// offset). A failed chunk keeps the session queued — the store keeps
    /// its resume offset, and a later heartbeat retries it — while the rest
    /// of the batch continues.
    ///
    /// The remaining count reported per session is `queue.len()` plus the
    /// batch tail not yet popped, so a label published after the k-th
    /// session includes the batch members still waiting their turn —
    /// without this, every mid-batch publish would under-count by the whole
    /// unpopped remainder and the dashboard would visibly step down only
    /// once per heartbeat.
    async fn process_batch(&mut self) {
        let mut batch: VecDeque<SessionId> = self.take_batch().into_iter().collect();
        while let Some(id) = batch.pop_front() {
            let finished = self.reindex_session(&id).await;
            if !finished {
                self.queue.insert(id);
            }
            // batch.len() is exactly the unpopped tail of this heartbeat's
            // batch (completed sessions are not re-inserted; a failed
            // session goes back into the queue, not into the tail).
            self.publish_remaining(batch.len()).await;
        }
    }

    /// Removes up to one batch's worth of sessions from the queue.
    fn take_batch(&mut self) -> Vec<SessionId> {
        (0..self.batch).map_while(|_| self.next_queued()).collect()
    }

    /// Removes and returns one queued session.
    fn next_queued(&mut self) -> Option<SessionId> {
        let id = self.queue.iter().next().cloned()?;
        self.queue.remove(&id);
        Some(id)
    }

    /// Reindexes one session to completion: chunk after chunk until the
    /// store reports the session fully indexed. Returns `false` when a
    /// chunk failed — the session did not finish and keeps its stored
    /// resume offset for the retry.
    async fn reindex_session(&self, id: &SessionId) -> bool {
        loop {
            match self
                .deps
                .services
                .session_store
                .reindex_session_chunk(id, REINDEX_CHUNK)
                .await
            {
                Ok(true) => {
                    tracing::debug!(session_id = %id, "FTS reindexed session");
                    return true;
                }
                Ok(false) => {} // chunk advanced; continue with the next chunk
                Err(report) => {
                    tracing::warn!(
                        session_id = %id,
                        error = ?report,
                        "FTS reindex chunk failed; resuming from its stored offset next heartbeat"
                    );
                    return false;
                }
            }
        }
    }

    /// Publishes the queue's remaining count: "N sessions pending", or —
    /// once the queue and the in-flight batch tail empty — the
    /// authoritative up-to-date check. A failed count publishes nothing;
    /// the previous message stays up and the next publish retries.
    async fn publish_remaining(&self, in_flight: usize) {
        let remaining = self.queue.len() + in_flight;
        if remaining == 0 {
            self.publish_up_to_date().await;
        } else {
            self.publish_status(format!("{remaining} sessions pending"))
                .await;
        }
    }

    /// Publishes the idle label after an empty queue read. An empty
    /// `dirty_session_ids` does not prove the fts dirty-marker table is
    /// clean: unreadable (non-UUID) markers are skipped silently by that
    /// query, so the authoritative `pending_dirty_count` is consulted
    /// before claiming "index up to date" — a nonzero count publishes the
    /// pending label instead and logs how many markers could not be read.
    async fn publish_up_to_date(&self) {
        match self.deps.services.session_store.pending_dirty_count().await {
            Ok(0) => self.publish_status("index up to date".to_owned()).await,
            Ok(n) => {
                tracing::warn!(
                    pending = n,
                    "dirty_session_ids returned an empty queue while the dirty-marker count is nonzero; some markers may be unreadable"
                );
                self.publish_status(format!("{n} sessions pending")).await;
            }
            Err(_) => {
                tracing::warn!("failed to read the pending dirty count; label not updated");
            }
        }
    }

    async fn publish_status(&self, status: String) {
        self.publish(jinn_slices::ServiceStatusUpdate {
            name: SEARCH_INDEX_ROW_NAME.to_owned(),
            description: None,
            lifecycle: None,
            status_message: Some(status),
        })
        .await;
    }

    /// Schedules the next heartbeat after this actor's interval. A failed
    /// send means the actor is stopping.
    fn reschedule(&self, ctx: &mut Context<SearchIndexActor, ()>) {
        let interval = self.interval;
        let actor_ref = ctx.actor_ref().clone();
        tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            let _ = actor_ref.tell(ReindexTick).send().await;
        });
    }
}

/// Spawns the actor as a supervised child of the root and returns its ref.
///
/// Kicks the first heartbeat via a detached task **after** the supervised
/// spawn handshake resolves, so the heartbeat never blocks startup: the
/// wiring moves on while the tick processes concurrently.
/// (`spawn_search_index_actor` remains the single registration point the
/// `spawn_tracked!` macro wraps.)
pub async fn spawn_search_index_actor(
    deps: SearchIndexActorDeps,
    supervisor: &crate::common::root_supervisor::RootSupervisorRef,
) -> ActorRef<SearchIndexActor> {
    let actor_ref = SearchIndexActor::supervise(supervisor, deps)
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;
    tokio::spawn({
        let actor_ref = actor_ref.clone();
        async move {
            // A failed send only means the actor is already stopping.
            let _ = actor_ref.tell(ReindexTick).send().await;
        }
    });
    actor_ref
}
