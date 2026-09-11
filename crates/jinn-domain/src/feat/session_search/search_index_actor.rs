//! Background actor that keeps the FTS search index fresh.
//!
//! Owns no `AppState` fields — its writes go to the database (the
//! `session_fts` index table), not to shared state. Dirty sessions are
//! recorded by triggers on the `sessions` table (schema v26); this actor
//! drives the drain itself: once at startup (backfill after an upgrade),
//! then on a fixed interval, reindexing one session per loop iteration and
//! publishing the remaining pending count to the `search-index` dashboard
//! row after every index operation. Search results may trail the newest
//! saves by one interval; the agent's own current-turn entries are in its
//! context regardless.

use std::time::Duration;

use kameo::actor::{ActorRef, Spawn};
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;

/// How often the actor drains the dirty set in production.
pub const REINDEX_INTERVAL: Duration = Duration::from_secs(5);

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
}

/// The search-index maintenance actor.
///
/// Statelessness is deliberate: the `fts_dirty` table is the durable record
/// of pending work, so a crash or a skipped drain costs freshness only, and
/// the work is retried on the next tick (or the next startup).
pub struct SearchIndexActor {
    deps: ActorDeps,
    interval: Duration,
}

impl kameo::Actor for SearchIndexActor {
    type Args = SearchIndexActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        // Kick the first drain immediately (startup backfill), then keep the
        // tick alive by rescheduling from the handler itself. A failed send
        // only means the actor is already stopping.
        let _ = actor_ref.tell(ReindexTick).send().await;
        Ok(Self {
            deps: args.deps,
            interval: args.interval,
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
    /// Drains the dirty set one session at a time, publishing dashboard
    /// progress after every index operation. Log-and-continue: a failed
    /// session is left dirty (durable pending work) and never blocks the
    /// rest of the batch. An empty batch publishes once too, so the row
    /// reads "index up to date" as soon as the actor is idle.
    async fn drain_once(&self) {
        let Ok(ids) = self.deps.services.session_store.dirty_session_ids().await else {
            tracing::warn!("failed to read dirty session markers; will retry next tick");
            return;
        };
        for id in &ids {
            match self.deps.services.session_store.reindex_session(id).await {
                Ok(()) => tracing::debug!(session_id = %id, "FTS reindexed session"),
                Err(report) => tracing::warn!(
                    session_id = %id,
                    error = ?report,
                    "FTS reindex failed; leaving marker dirty for the next drain"
                ),
            }
            self.publish_progress().await;
        }
        if ids.is_empty() {
            self.publish_progress().await;
        }
    }

    /// Publishes the live remaining pending count to the `search-index`
    /// dashboard row: "N sessions pending", or "index up to date" once the
    /// queue drains. A failed count publishes nothing — the previous message
    /// stays up and the next session's publish retries.
    async fn publish_progress(&self) {
        let status = match self.deps.services.session_store.pending_dirty_count().await {
            Ok(0) => "index up to date".to_owned(),
            Ok(n) => format!("{n} sessions pending"),
            Err(_) => return,
        };
        self.publish(crate::feat::dashboard::ServiceStatusUpdate {
            name: SEARCH_INDEX_ROW_NAME.to_owned(),
            description: None,
            lifecycle: None,
            status_message: Some(status),
        })
        .await;
    }
}

/// Spawns the actor as a supervised child of the root and returns its ref.
pub async fn spawn_search_index_actor(
    deps: SearchIndexActorDeps,
    supervisor: &crate::common::root_supervisor::RootSupervisorRef,
) -> ActorRef<SearchIndexActor> {
    SearchIndexActor::supervise(supervisor, deps)
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await
}
