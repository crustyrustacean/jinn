//! The prune-mutation shape every auto-prune strategy shares.
//!
//! A strategy's heuristic is its own work, but the mutation it emits when that
//! heuristic decides to prune is not: every strategy builds the same
//! `SetContextOverride` with the same `ChangeSource::Worker` naming the
//! emitting worker. That shape lives here so the source field cannot drift
//! between strategies, and a strategy file never restates it.
//!
//! Two parts of the worker skeleton deliberately stay per-strategy. The
//! configuration handle is a field on each worker (nine declare it `pub`,
//! `regex` keeps it private because nothing outside its own `compile` reads
//! it), and each worker implements `HistoryWorker::name` and `evaluate`
//! against the trait in [`crate::worker`].
//!
//! Neither is worth factoring, and the reasons are worth stating. `evaluate`
//! is the heuristic itself — factoring it would leave the shared part doing
//! nothing but calling back into the strategy. `name` returns a distinct
//! string per strategy and the attribute silencing lifetime-elision on it is
//! repeated verbatim, but a ten-line macro or a shared base trait would buy
//! one deduplicated line at the cost of indirection over the trait that is
//! the whole point of the type. The one genuinely shared piece, the
//! enablement read, is [`crate::worker::strategy_section`], and it already
//! lives beside the trait it serves.

use jinn_core_types::{ChangeSource, ChatEntryId, ContextOverride, HistoryMutation};

/// The mutation that sets an entry's context override and records the worker
/// that decided it.
///
/// Every strategy emits this same shape — the override value plus a source
/// naming the emitting worker — so the source field cannot drift between
/// strategies, and a strategy file never restates it.
#[must_use]
pub(super) fn override_mutation(
    entry_id: &ChatEntryId,
    value: ContextOverride,
    worker_name: &str,
) -> HistoryMutation {
    HistoryMutation::SetContextOverride {
        entry_id: entry_id.clone(),
        value,
        source: ChangeSource::Worker {
            name: worker_name.to_owned(),
        },
    }
}

/// The mutation that forces an entry out of the context window.
///
/// The common case within [`override_mutation`]: a prune.
#[must_use]
pub(super) fn prune_mutation(entry_id: &ChatEntryId, worker_name: &str) -> HistoryMutation {
    override_mutation(entry_id, ContextOverride::ForcedExclude, worker_name)
}
