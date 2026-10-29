//! Background actor that prunes stale session trees and shrinks the database.
//!
//! A trouper [`ServiceActor`] driven by a self-addressed tick, following the
//! same detached-sleep idiom as the search-index actor (trouper has no
//! scheduler): the first tick self-schedules 30 seconds after spawn, and each
//! tick's **last** act is to schedule the next one an hour after the work
//! completed. Sweeps are therefore spaced from completion, not per
//! wall-clock hour.
//!
//! Owns no `AppState` fields, no cell, no routes — its writes go to the
//! store (`delete_many`, `incremental_vacuum`) and to the log. On each tick
//! it reads the `[maintenance]` section of `jinn.toml`; when
//! `session_retention` is absent or unparseable the sweep is a no-op (jinn
//! never deletes anything the user has not explicitly told it to), but the
//! tick still reschedules so filling the section in takes effect without a
//! restart.
//!
//! Deletion is by **session tree**: a root (or pre-orphaned child acting as
//! its own root) and every descendant qualify together, and only when
//! *every* member is archived, outside the Library, and the tree's newest
//! `updated_at` is older than the retention threshold. One fresh member
//! protects its whole family. The bulk delete is one transaction in the
//! store, so a pruned tree vanishes whole; a bounded incremental vacuum
//! afterwards hands the freed pages back to the filesystem (the database is
//! converted to incremental auto-vacuum by schema migration v32).

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
pub const INITIAL_DELAY: Duration = Duration::from_secs(30);

/// How long after a sweep's work completes the next one is scheduled.
pub const SWEEP_INTERVAL: Duration = Duration::from_hours(1);

/// The most pages one incremental vacuum pass may hand back to the OS.
///
/// Bounded so a sweep never holds the database in a long vacuum: a few
/// hundred 4 KiB pages per hour is plenty for normal pruning, and an
/// oversized backlog (e.g. the first sweep after enabling retention on an
/// old database) drains across sweeps, since `incremental_vacuum` returns
/// as soon as the freelist is empty anyway.
pub const MAX_VACUUM_PAGES: u32 = 512;

/// The database maintenance actor's static trouper path.
pub const DB_MAINTENANCE_PATH: &str = "db-maintenance";

/// Dependencies for [`DbMaintenanceActor`].
#[derive(Clone)]
pub struct DbMaintenanceActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// Delay before the first sweep. Production uses [`INITIAL_DELAY`];
    /// tests inject a small value so convergence assertions don't wait on
    /// the default.
    pub initial_delay: Duration,
    /// Interval between sweeps, measured from the previous sweep's
    /// completion. Production uses [`SWEEP_INTERVAL`]; tests inject
    /// milliseconds.
    pub sweep_interval: Duration,
}

/// The database maintenance actor: sweep, vacuum, reschedule.
pub struct DbMaintenanceActor {
    deps: ActorDeps,
    /// The system this actor runs on — captured at spawn so the
    /// reschedule can self-address. (The services container may carry a
    /// different system in tests, where the harness spawns on its own.)
    system: ActorSystem,
    sweep_interval: Duration,
}

impl ServiceActor for DbMaintenanceActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "trait contract: start is never called (spawn uses start_with)"
    )]
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the deps via `start_with`
        // (ActorDeps carries typed handles that cannot ride JSON args).
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("DbMaintenanceActor is spawned via start_with"),
        )
    }
}

impl DbMaintenanceActor {
    /// Spawns the actor at its static trouper path and returns the path.
    ///
    /// The first tick is kicked by a detached task after `initial_delay`
    /// (not immediately — the sweep must not compete with startup); every
    /// later tick is scheduled by the previous one's final act.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(system: &ActorSystem, deps: DbMaintenanceActorDeps) -> ActorPath {
        let path = ActorPath::new(DB_MAINTENANCE_PATH);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                let system = system.clone();
                move || {
                    let deps = deps.clone();
                    let system = system.clone();
                    Box::pin(async move {
                        Ok(Self {
                            deps: deps.deps,
                            system,
                            sweep_interval: deps.sweep_interval,
                        })
                    })
                }
            })
            .handles::<MaintenanceTick>()
            .mailbox(8, trouper::inbox::OverloadPolicy::Block)
            .start();
        // Kick the first sweep after the initial delay. A failed send only
        // means the actor is already stopping.
        let kicker = system.clone();
        let kick_path = path.clone();
        let initial_delay = deps.initial_delay;
        tokio::spawn(async move {
            tokio::time::sleep(initial_delay).await;
            let _ = kicker.tell(kick_path, MaintenanceTick).await;
        });
        path
    }

    /// Delivers the next sweep to this actor after `sweep_interval`.
    ///
    /// Called as the tick handler's **last** act — after the sweep's work
    /// (or no-op) completed — so intervals are measured from completion.
    fn reschedule(&self) {
        let system = self.system.clone();
        let path = ActorPath::new(DB_MAINTENANCE_PATH);
        let interval = self.sweep_interval;
        tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            let _ = system.tell(path, MaintenanceTick).await;
        });
    }
}

/// A self-addressed sweep: prune stale trees, vacuum, then reschedule.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, trouper::schema::Command)]
#[schema(
    description = "Database maintenance sweep: prune stale session trees, then incremental-vacuum."
)]
pub struct MaintenanceTick;

impl MsgHandler<MaintenanceTick> for DbMaintenanceActor {
    async fn handle(&mut self, _msg: &MaintenanceTick, _ctx: &mut MsgCtx<'_>) {
        let retention = self.retention().await;
        if let Some(retention) = retention {
            self.sweep(retention).await;
        }
        // The chain must stay alive even when disabled or failed: the last
        // act of every tick, without exception, is the next tick.
        self.reschedule();
    }
}

impl DbMaintenanceActor {
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

    /// One sweep: select stale trees, delete them whole, vacuum, log.
    async fn sweep(&self, retention: Duration) {
        let summaries = match self.deps.services.session_store.load_summaries().await {
            Ok(summaries) => summaries,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "maintenance sweep could not read session summaries; skipping this sweep"
                );
                return;
            }
        };

        let now = jiff::Timestamp::now();
        let stale = prune_candidates(&summaries, now, retention);
        if stale.is_empty() {
            return;
        }

        if let Err(error) = self.deps.services.session_store.delete_many(&stale).await {
            tracing::warn!(
                error = ?error,
                count = stale.len(),
                "maintenance sweep failed to delete stale sessions; the next sweep retries"
            );
            return;
        }
        // Freed pages go back to the filesystem within the bounded budget.
        if let Err(error) = self
            .deps
            .services
            .session_store
            .incremental_vacuum(MAX_VACUUM_PAGES)
            .await
        {
            tracing::warn!(
                error = ?error,
                "maintenance sweep deleted sessions but the incremental vacuum failed; pages return on a later sweep"
            );
        }
        tracing::info!(
            count = stale.len(),
            ?retention,
            "maintenance sweep pruned stale session trees"
        );
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
