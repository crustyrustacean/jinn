//! Background actor that keeps the FTS search index fresh.
//!
//! Owns no `AppState` fields — its writes go to the database (the
//! `session_fts` index table), not to shared state. Dirty sessions are
//! recorded by triggers on the `sessions` table (schema v26); this actor
//! drains them: once at startup (backfill after an upgrade) and then on a
//! fixed interval. Search results may trail the newest saves by one interval;
//! the agent's own current-turn entries are in its context regardless.

use std::time::Duration;

use kameo::actor::{ActorRef, Spawn};
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::ActorDeps;

/// How often the actor drains the dirty set in production.
pub const REINDEX_INTERVAL: Duration = Duration::from_secs(5);

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
        // tick alive by rescheduling from the handler itself.
        actor_ref.tell(ReindexTick).send().await.ok();
        Ok(Self {
            deps: args.deps,
            interval: args.interval,
        })
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
            actor_ref.tell(ReindexTick).send().await.ok();
        });
    }
}

impl SearchIndexActor {
    /// Drains the dirty set once. Log-and-continue: a failed drain leaves the
    /// markers set (durable pending work) and the next tick retries.
    async fn drain_once(&self) {
        match self
            .deps
            .services
            .session_store
            .reindex_dirty_sessions()
            .await
        {
            Ok(0) => {}
            Ok(count) => {
                tracing::debug!(sessions = count, "FTS reindex drained dirty sessions");
            }
            Err(report) => {
                tracing::warn!(error = ?report, "FTS reindex drain failed; will retry next tick");
            }
        }
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
