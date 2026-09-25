//! The context-curation slice — what the LLM sees each turn.
//!
//! Owns the curation workers: the auto-prune strategies (exclude
//! stale/redundant entries) and the compaction worker (summarize old
//! history into one checkpoint entry). Both produce `HistoryMutation`
//! batches and publish them as `SubmitHistoryMutations`; the kernel
//! session actor's accumulation gate stays the sole applier.
//!
//! Two trouper [`ServiceActor`]s, fed by the `jinn.context-curation`
//! forward route: the prune actor (evaluates the enabled strategies on
//! each `HistoryAppended`, snapshotting the history internally) and the
//! compaction actor (runs `CompactionWorker` on `TriggerCompaction`).
//!
//! Kernel dependency (see Cargo.toml): the compaction worker reads
//! through `State` and consumes the kernel token estimator, granted at
//! slice activation.

pub mod compaction_actor;
pub mod compaction_algorithm;
pub mod compaction_serializer;
pub mod compaction_worker;
pub mod prune_actor;
pub mod strategies;
pub mod worker;

pub use strategies::min_age;

use jinn_slices::SliceHost;

/// Activates the slice: spawns the prune + compaction actors on trouper
/// (their `.subscribe` declarations are the readiness point).
///
/// # Panics
///
/// Panics if any worker's trouper spawn or topic subscription fails —
/// both are wiring bugs that must abort composition.
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    prune_workers: Vec<Box<dyn worker::HistoryWorker>>,
    compaction_deps: compaction_actor::CompactionActorDeps,
) {
    let prune_path = prune_actor::PruneActor::spawn(
        host.system(),
        compaction_deps.state.clone(),
        compaction_deps.services.clone(),
        prune_workers,
    );
    drop(prune_path);
    drop(compaction_actor::CompactionActor::spawn(
        host.system(),
        compaction_deps,
    ));
}
