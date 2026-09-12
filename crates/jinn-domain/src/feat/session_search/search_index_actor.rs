//! Background actor that keeps the FTS search index fresh.
//!
//! Owns no `AppState` fields — its writes go to the database (the
//! `session_fts` index table), not to shared state. Dirty sessions are
//! recorded by triggers on the `sessions` table (schema v26); this actor
//! drives the drain itself: once at startup (backfill after an upgrade),
//! then on a fixed interval, reindexing sessions one at a time and
//! publishing the remaining pending count to the `search-index` dashboard
//! row around every drain. Search results may trail the newest saves by one
//! interval; the agent's own current-turn entries are in its context
//! regardless.
//!
//! Each tick's drain is bounded by a time budget ([`REINDEX_BUDGET`] in
//! production) so a large pending queue (e.g. the first-launch backfill
//! after the schema upgrade, hundreds of sessions) drains cooperatively:
//! the tick handler returns promptly, the actor stays stoppable, and
//! session persists interleave with backfill writes instead of starving
//! behind them. The `fts_dirty` table is the durable queue, so a budget
//! that expires simply resumes on the next tick.

use std::time::Duration;

use kameo::actor::{ActorRef, Spawn};
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;

/// How often the actor drains the dirty set in production.
pub const REINDEX_INTERVAL: Duration = Duration::from_secs(5);

/// How long one tick's drain may reindex before yielding to the next tick.
///
/// Bounds the per-tick write-lock hold so the drain never monopolizes the
/// database: startup stays responsive, the tick handler returns inside
/// shutdown timeouts, and session persists interleave with backfill work.
/// The store chunks each session's rebuild ([`REINDEX_CHUNK`] entries per
/// transaction), so the budget always bites within a bounded interval even
/// when a single session is enormous.
pub const REINDEX_BUDGET: Duration = Duration::from_secs(2);

/// How many entries one reindex chunk transaction may index.
///
/// Small enough that a debug-build parse of a chunk (the expensive part —
/// kind JSON of full tool outputs) stays well under the budget and the
/// write lock is released between chunks for concurrent persists.
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
    /// Poll interval. Production uses [`REINDEX_INTERVAL`]; tests inject a
    /// small value so convergence assertions don't wait on the default.
    pub interval: Duration,
    /// Per-tick drain time budget. Production uses [`REINDEX_BUDGET`]; tests
    /// inject `Duration::ZERO` (exactly one session per tick) or a large
    /// value (drain everything in one tick).
    pub budget: Duration,
}

/// The search-index maintenance actor.
///
/// Statelessness is deliberate: the `fts_dirty` table is the durable record
/// of pending work, so a crash or a skipped drain costs freshness only, and
/// the work is retried on the next tick (or the next startup).
pub struct SearchIndexActor {
    deps: ActorDeps,
    interval: Duration,
    budget: Duration,
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
        // kicks the first tick after the handshake instead.
        let _ = actor_ref; // unused without the kick; keeps the signature stable
        Ok(Self {
            deps: args.deps,
            interval: args.interval,
            budget: args.budget,
        })
    }
}

impl BusPublish for SearchIndexActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

/// A self-addressed tick that triggers one drain and schedules the next.
#[derive(Debug)]
pub struct ReindexTick;

impl Message<ReindexTick> for SearchIndexActor {
    type Reply = ();

    async fn handle(&mut self, _msg: ReindexTick, ctx: &mut Context<Self, Self::Reply>) {
        self.drain_once().await;
        // Schedule the next drain after this actor's interval. A failed send
        // means the actor is stopping.
        let interval = self.interval;
        let actor_ref = ctx.actor_ref().clone();
        tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            let _ = actor_ref.tell(ReindexTick).send().await;
        });
    }
}

impl SearchIndexActor {
    /// Drains the pending reindex queue within this tick's time budget,
    /// then reports the remaining pending count to the dashboard.
    ///
    /// Work is chunked ([`REINDEX_CHUNK`] entries per transaction) and
    /// resumable: the store's dirty-marker row records how far the current
    /// session's rebuild has progressed, so a budget that expires mid-session
    /// resumes exactly there — next tick or next launch. Publishes before the
    /// first chunk so the row reflects the queue immediately. Log-and-continue:
    /// a failed chunk is left at its old resume point and retried on a later
    /// tick. An empty queue publishes once, so the row reads "index up to
    /// date" as soon as the actor is idle.
    async fn drain_once(&self) {
        // Snapshot the queue once per tick: a failed chunk stays in place
        // (resumed by a later tick) instead of spinning the loop.
        let Ok(ids) = self.deps.services.session_store.dirty_session_ids().await else {
            tracing::warn!("failed to read the reindex queue; will retry next tick");
            return;
        };
        if ids.is_empty() {
            self.publish_up_to_date().await;
            return;
        }
        self.publish_progress().await;
        let deadline = tokio::time::Instant::now() + self.budget;
        for id in &ids {
            match self
                .deps
                .services
                .session_store
                .reindex_session_chunk(id, REINDEX_CHUNK)
                .await
            {
                Ok(true) => tracing::debug!(session_id = %id, "FTS reindexed session"),
                Ok(false) => tracing::debug!(
                    session_id = %id,
                    "FTS reindex chunk advanced the session's resume point"
                ),
                Err(report) => tracing::warn!(
                    session_id = %id,
                    error = ?report,
                    "FTS reindex chunk failed; resuming from its stored offset next tick"
                ),
            }
            if tokio::time::Instant::now() >= deadline {
                self.publish_progress().await;
                tracing::debug!("FTS reindex budget exhausted; resuming next tick");
                return;
            }
        }
        // Budget not exhausted: the queue just drained. Report the idle state.
        self.publish_progress().await;
    }

    /// Publishes the live remaining pending count to the `search-index`
    /// dashboard row: "N sessions pending", or "index up to date" once the
    /// queue drains. A failed count publishes nothing — the previous message
    /// stays up and the next publish retries.
    async fn publish_progress(&self) {
        if let Some(status) = self.pending_label().await {
            self.publish_status(status).await;
        }
    }

    /// Publishes "index up to date" unconditionally (the caller just observed
    /// an empty queue — a count round-trip would only race new dirt).
    async fn publish_up_to_date(&self) {
        self.publish_status("index up to date".to_owned()).await;
    }

    async fn pending_label(&self) -> Option<String> {
        match self.deps.services.session_store.pending_dirty_count().await {
            Ok(0) => Some("index up to date".to_owned()),
            Ok(n) => Some(format!("{n} sessions pending")),
            Err(_) => None,
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
}

/// Spawns the actor as a supervised child of the root and returns its ref.
///
/// Kicks the first drain via a detached task **after** the supervised spawn
/// handshake resolves, so the drain never blocks startup: the wiring moves
/// on while the tick processes concurrently. (`spawn_search_index_actor`
/// remains the single registration point the `spawn_tracked!` macro wraps.)
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
