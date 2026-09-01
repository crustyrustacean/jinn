//! Derivation of currently-applied auto-prune savings from entry state.
//!
//! [`prune_report`] scans the active session's history at render time and
//! totals the token cost of entries that are currently excluded by an
//! auto-prune worker — `ContextOverride::ForcedExclude` whose latest
//! `context_history` event names a pruner worker (compaction excluded).
//! No state is written: the quake bar recomputes this per frame, so the
//! number always reflects the live history (re-includes drop out
//! automatically and pending, un-flushed prunes never appear).

use crate::feat::session::chat_entry::{ChangeSource, ChatEntry, ContextOverride};
use crate::feat::session::entry_token_cache::EntryTokenCache;

/// The `HistoryWorker::name` of the compaction worker.
///
/// Compaction records `ForcedExclude` + `Worker { name: "compaction" }` — the
/// same shape auto-pruners use — so the name filter is what keeps compaction
/// out of the pruned total.
const COMPACTION_WORKER_NAME: &str = "compaction";

/// Result of scanning history for currently-applied auto-pruner exclusions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Sum of cached token counts over counted entries (lower bound when
    /// counts are missing).
    pub tokens: u64,
    /// Number of counted entries.
    pub entries: usize,
}

/// Totals the entries currently excluded by auto-prune workers.
///
/// An entry is counted iff its override is still
/// [`ContextOverride::ForcedExclude`] and its most recent
/// `context_history` event came from a `ChangeSource::Worker` other than
/// compaction. Missing token-cache counts contribute zero tokens but the
/// entry still counts, so `tokens` is a lower bound. Entries whose prune is
/// still buffered in the mutation accumulator are not applied yet and are
/// never counted.
pub fn prune_report(history: &[ChatEntry], token_cache: &EntryTokenCache) -> PruneReport {
    let tokens = history
        .iter()
        .filter(|entry| is_pruned_by_worker(entry))
        .map(|entry| u64::from(token_cache.get(&entry.id).unwrap_or(0)))
        .sum();

    let entries = history
        .iter()
        .filter(|entry| is_pruned_by_worker(entry))
        .count();

    PruneReport { tokens, entries }
}

/// Whether the entry is currently excluded by an auto-prune worker.
///
/// Counts only entries whose override is still `ForcedExclude` (a later
/// user re-include drops them out) and whose latest recorded context
/// change names a pruner worker — never compaction, never the user, and
/// never an unattributed exclude with no recorded history.
fn is_pruned_by_worker(entry: &ChatEntry) -> bool {
    let Some(last) = entry.context_history.last() else {
        return false;
    };

    entry.context_override() == ContextOverride::ForcedExclude
        && matches!(
            &last.source,
            ChangeSource::Worker { name } if name != COMPACTION_WORKER_NAME
        )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::feat::session::mutation_accumulator::MutationAccumulator;

    const PRUNER_NAME: &str = "edit_read";

    fn worker_prune(entry: &mut ChatEntry) {
        entry.apply_context_override(
            ContextOverride::ForcedExclude,
            ChangeSource::Worker {
                name: PRUNER_NAME.to_owned(),
            },
        );
    }

    #[rstest::rstest]
    #[test]
    fn counts_worker_excluded_entry_tokens() {
        // Given a worker-excluded entry with a cached token count.
        let mut entry = ChatEntry::user("big tool output");
        worker_prune(&mut entry);
        let mut cache = EntryTokenCache::default();
        cache.insert(entry.id.clone(), 300);

        // When computing the prune report.
        let report = prune_report(std::slice::from_ref(&entry), &cache);

        // Then the entry's tokens are counted.
        assert_eq!(report.tokens, 300);
        // And one entry is counted.
        assert_eq!(report.entries, 1);
    }

    #[rstest::rstest]
    #[test]
    fn skips_compaction_sourced_excludes() {
        // Given an entry excluded by compaction.
        let mut entry = ChatEntry::user("summarized away");
        entry.apply_context_override(
            ContextOverride::ForcedExclude,
            ChangeSource::Worker {
                name: COMPACTION_WORKER_NAME.to_owned(),
            },
        );
        let mut cache = EntryTokenCache::default();
        cache.insert(entry.id.clone(), 300);

        // When computing the prune report.
        let report = prune_report(std::slice::from_ref(&entry), &cache);

        // Then the compaction exclude is not counted.
        assert_eq!(report.tokens, 0);
        assert_eq!(report.entries, 0);
    }

    #[rstest::rstest]
    #[test]
    fn skips_user_sourced_excludes() {
        // Given a user-excluded entry.
        let mut entry = ChatEntry::user("user ignored this");
        entry.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::User);
        let mut cache = EntryTokenCache::default();
        cache.insert(entry.id.clone(), 300);

        // When computing the prune report.
        let report = prune_report(std::slice::from_ref(&entry), &cache);

        // Then the user exclude is not counted.
        assert_eq!(report.tokens, 0);
        assert_eq!(report.entries, 0);
    }

    #[rstest::rstest]
    #[test]
    fn drops_reincluded_entries() {
        // Given a worker-excluded entry that the user later re-included.
        let mut entry = ChatEntry::user("brought back");
        worker_prune(&mut entry);
        entry.apply_context_override(ContextOverride::ForcedInclude, ChangeSource::User);
        let mut cache = EntryTokenCache::default();
        cache.insert(entry.id.clone(), 300);

        // When computing the prune report.
        let report = prune_report(std::slice::from_ref(&entry), &cache);

        // Then the re-included entry is not counted.
        assert_eq!(report.tokens, 0);
        assert_eq!(report.entries, 0);
    }

    #[rstest::rstest]
    #[test]
    fn skips_pending_only_prunes() {
        // Given an entry still at its default override whose prune is buffered
        // only in an accumulator.
        let entry = ChatEntry::user("not yet flushed");
        let mut accumulator = MutationAccumulator::default();
        accumulator.push(
            entry.id.clone(),
            ContextOverride::ForcedExclude,
            ChangeSource::Worker {
                name: PRUNER_NAME.to_owned(),
            },
            300,
        );

        // When computing the prune report from history and the token cache.
        let cache = EntryTokenCache::default();
        let report = prune_report(std::slice::from_ref(&entry), &cache);

        // Then the pending-only prune is not counted — the accumulator is
        // invisible to the derivation.
        assert_eq!(report.tokens, 0);
        assert_eq!(report.entries, 0);
    }

    #[rstest::rstest]
    #[test]
    fn missing_token_count_contributes_zero_but_entry_counts() {
        // Given a worker-excluded entry with no cached token count.
        let mut entry = ChatEntry::user("uncounted");
        worker_prune(&mut entry);
        let cache = EntryTokenCache::default();

        // When computing the prune report.
        let report = prune_report(std::slice::from_ref(&entry), &cache);

        // Then the tokens are a lower bound (0).
        assert_eq!(report.tokens, 0);
        // And the entry still counts.
        assert_eq!(report.entries, 1);
    }

    #[rstest::rstest]
    #[test]
    fn empty_history_yields_zero_report() {
        // Given no history and an empty cache.
        let cache = EntryTokenCache::default();

        // When computing the prune report.
        let report = prune_report(&[], &cache);

        // Then both totals are zero.
        assert_eq!(report.tokens, 0);
        assert_eq!(report.entries, 0);
    }

    #[rstest::rstest]
    #[test]
    fn unattributed_exclude_not_counted() {
        // Given a ForcedExclude entry with an empty context-history audit trail.
        let entry = ChatEntry::user("restored without audit")
            .with_context_override(ContextOverride::ForcedExclude);
        let mut cache = EntryTokenCache::default();
        cache.insert(entry.id.clone(), 300);

        // When computing the prune report.
        let report = prune_report(std::slice::from_ref(&entry), &cache);

        // Then the unattributed exclude is not counted.
        assert_eq!(report.tokens, 0);
        assert_eq!(report.entries, 0);
    }
}
