//! Tool-age-window auto-prune worker.
//!
//! Keeps only the most recent `max_age_entries` (default: 100) entries' worth
//! of tool activity. Any `ToolCall`/`ToolResult` pair older than that window
//! is marked [`ForcedExclude`] so the LLM never sees stale tool output.
//!
//! # Semantics
//!
//! - The threshold counts every entry in raw history — already-excluded,
//!   thinking, transient, system, error, and pending-result entries all
//!   count. This makes the window position independent of what other
//!   auto-prune workers have already `ForcedExclude`d, so multiple workers
//!   compose cleanly: each worker's prune region is fixed by raw history
//!   length alone, not by what has already been `ForcedExclude`d by other
//!   workers.
//! - The window is measured from the end of history backward. Once
//!   `max_age_entries` entries have been counted, everything older is in
//!   the prune window.
//! - A `ToolCall`/`ToolResult` pair is pruned **atomically**: when the
//!   `ToolCall` is in the prune window, both halves are excluded. Splitting
//!   a pair corrupts the LLM context (providers reject orphaned results).
//!   By history ordering the call always precedes the result, so a call in
//!   the prune window implies the result is at or beyond the call; both are
//!   pruned together.
//! - Pending/orphaned pairs (no matching `ToolResult`, or a `ToolResult`
//!   whose status is `Pending`) are never pruned — see Gotcha #2 in the
//!   plan.
//! - Already-excluded entries do not receive duplicate
//!   `SetContextOverride` mutations.
//!
//! # Example (max_age_entries = 4)
//!
//! ```text
//! X  [User]                  ← index 0 (pruned: too old)
//! X  [Tool Call]: bash       ← index 1 (pruned: call in prune window)
//! X  [Tool Result] (OK)      ← index 2 (pruned atomically with its call)
//! X  [Assistant]             ← index 3 (pruned: too old)
//!    [User]                  ← index 4 (kept: within window)
//!    [Tool Call]: bash       ← index 5 (kept)
//!    [Tool Result] (OK)      ← index 6 (kept)
//!    [Assistant]             ← index 7 (kept)
//! ```
//!
//! [`ForcedExclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedExclude

use std::sync::Arc;

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::ToolAgeWindowAutoPruneConfig;
use crate::feat::session::chat_entry::{ChangeSource, ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::SessionId;

/// Tool-age-window auto-prune worker.
///
/// See module docs for full semantics. Construct with
/// [`ToolAgeWindowAutoPruneConfig`]; `max_age_entries` is clamped to a
/// minimum of 1 at evaluation time.
#[derive(Clone)]
pub struct ToolAgeWindowAutoPruneWorker {
    /// Configuration for the tool-age-window auto-prune strategy.
    pub config: ToolAgeWindowAutoPruneConfig,
}

/// Walk forward from a `ToolCall` to find its matching non-pending
/// `ToolResult` by tool call id.
///
/// Returns `None` if:
/// - no matching result exists (orphaned call), or
/// - the matching result has status `Pending` (incomplete pair — pruning
///   the call would orphan the future result and corrupt the next provider
///   request).
///
/// This is the same shape as the helpers in `broken_edit`, `consecutive_reads`,
/// and `regex`, but with an explicit `Pending` guard (see Gotcha #2).
fn find_completed_matching_result(
    history: &[ChatEntry],
    call_idx: usize,
    tool_call_id: &str,
) -> Option<ChatEntryId> {
    // ToolResults appear after their ToolCall, so scan forward only.
    for entry in history.iter().skip(call_idx + 1) {
        if let ChatEntryKind::ToolResult { id, status, .. } = &entry.kind
            && id == tool_call_id
        {
            // Skip pending results — the pair is incomplete.
            if *status == ToolResultStatus::Pending {
                return None;
            }
            return Some(entry.id.clone());
        }
    }
    // No matching result found — the call is still pending or orphaned.
    None
}

/// Compute the index of the first entry inside the keep window.
///
/// The keep window is the last `max_age` entries in raw history, regardless
/// of whether each entry is currently in LLM context. Counting every entry
/// (rather than only `is_in_context()` entries) makes the window position
/// independent of decisions made by other auto-prune workers, so the
/// workers compose cleanly: each worker's prune region is fixed by raw
/// history length alone, not by what has already been `ForcedExclude`d.
///
/// `max_age` is clamped to a minimum of 1.
///
/// Returns `None` if `history.len() < max_age` (nothing to prune).
fn compute_keep_window_start(history: &[ChatEntry], max_age: usize) -> Option<usize> {
    let max_age = max_age.max(1);
    if history.len() < max_age {
        return None;
    }
    Some(history.len() - max_age)
}

/// Build the list of `SetContextOverride::ForcedExclude` mutations for a
/// single snapshot.
///
/// Pure function (no `&self`) so unit tests can call it directly without
/// spinning up a tokio runtime.
///
/// Algorithm:
/// 1. Find the keep window start index (the `max_age`-th entry from the
///    end of raw history, regardless of in-context status).
/// 2. For every `ToolCall` at an index strictly less than the keep window
///    start, attempt to find its completed matching result.
/// 3. If found, emit `ForcedExclude` mutations for both halves — unless an
///    individual half is already excluded.
///
/// Pair-atomicity across the cutoff is preserved by the forward scan in
/// `find_completed_matching_result`: even if the result lives at an index
/// `>= keep_window_start`, it is still found and excluded together with
/// its call.
fn build_age_window_mutations(
    history: &[ChatEntry],
    max_age: usize,
    worker_name: &str,
) -> Vec<HistoryMutation> {
    let Some(keep_window_start) = compute_keep_window_start(history, max_age) else {
        // Fewer than max_age entries — nothing to prune.
        return Vec::new();
    };

    let mut mutations = Vec::new();

    for i in 0..keep_window_start {
        let entry = &history[i];

        // Only ToolCalls in the prune region are candidates.
        let tool_call_id = match &entry.kind {
            ChatEntryKind::ToolCall { id, .. } => id.clone(),
            _ => continue,
        };

        let call_id = entry.id.clone();
        let call_protected = entry.is_protected_from_prune();

        // Find the matching non-pending result. If none (orphaned or still
        // pending), skip the entire pair.
        let Some(result_id) = find_completed_matching_result(history, i, &tool_call_id) else {
            continue;
        };

        // Locate the result entry to check its exclude state. Forward scan
        // from i+1 — guaranteed to find it because find_completed_matching_result
        // just did.
        let result_protected = history
            .iter()
            .skip(i + 1)
            .find(|e| e.id == result_id)
            .is_some_and(ChatEntry::is_protected_from_prune);

        // Emit mutations only for halves not protected from prune.
        if !call_protected {
            tracing::debug!(
                entry_id = %call_id,
                keep_window_start,
                "tool_age_window: excluding old tool call"
            );
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: call_id,
                value: ContextOverride::ForcedExclude,
                source: ChangeSource::Worker {
                    name: worker_name.to_owned(),
                },
            });
        }
        if !result_protected {
            tracing::debug!(
                entry_id = %result_id,
                keep_window_start,
                "tool_age_window: excluding old tool result"
            );
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: result_id,
                value: ContextOverride::ForcedExclude,
                source: ChangeSource::Worker {
                    name: worker_name.to_owned(),
                },
            });
        }
    }

    mutations
}

#[async_trait::async_trait]
impl HistoryWorker for ToolAgeWindowAutoPruneWorker {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "auto-prune-tool-age-window"
    }

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        let mutations = build_age_window_mutations(&history, self.config.max_age_entries, self.name());
        tracing::debug!(
            mutations = mutations.len(),
            max_age_entries = self.config.max_age_entries,
            history_len = history.len(),
            "tool_age_window evaluate done"
        );
        mutations
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::preferences_actor::user_preferences::ToolAgeWindowAutoPruneConfig;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::protocol::SessionId;

    /// Build a worker with the given `max_age_entries` (enabled = true).
    fn worker(max_age: usize) -> ToolAgeWindowAutoPruneWorker {
        ToolAgeWindowAutoPruneWorker {
            config: ToolAgeWindowAutoPruneConfig {
                enabled: true,
                max_age_entries: max_age,
            },
        }
    }

    /// Build N plain user entries (all in-context).
    fn users(n: usize) -> Vec<ChatEntry> {
        (0..n)
            .map(|i| ChatEntry::user(format!("user msg {i}")))
            .collect()
    }

    /// Build a bash ToolCall + successful ToolResult pair.
    fn bash_pair(call_id: &str, command: &str, output: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "bash", format!(r#"{{"command": "{command}"}}"#)),
            ChatEntry::tool_result(call_id, "bash", output, ToolResultStatus::Success),
        ]
    }

    /// Build a bash ToolCall + pending ToolResult pair.
    fn bash_pending_pair(call_id: &str, command: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "bash", format!(r#"{{"command": "{command}"}}"#)),
            ChatEntry::tool_result(call_id, "bash", "", ToolResultStatus::Pending),
        ]
    }

    /// Evaluate the worker on a history snapshot.
    fn evaluate(w: &ToolAgeWindowAutoPruneWorker, history: Vec<ChatEntry>) -> Vec<HistoryMutation> {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let history: Arc<[ChatEntry]> = history.into();
        rt.block_on(async { w.evaluate(&SessionId::new(), history).await })
    }

    /// Collect the entry ids targeted by `SetContextOverride::ForcedExclude` mutations.
    fn excluded_ids(mutations: &[HistoryMutation]) -> std::collections::HashSet<ChatEntryId> {
        let mut out = std::collections::HashSet::new();
        for m in mutations {
            if let HistoryMutation::SetContextOverride {
                entry_id,
                value: ContextOverride::ForcedExclude,
                ..
            } = m
            {
                out.insert(entry_id.clone());
            }
        }
        out
    }

    // ------------------------------------------------------------------
    // 1. empty_history_produces_no_mutations
    // ------------------------------------------------------------------
    #[test]
    fn empty_history_produces_no_mutations() {
        let w = worker(100);
        assert!(evaluate(&w, Vec::new()).is_empty());
    }

    // ------------------------------------------------------------------
    // 2. history_under_threshold_produces_no_mutations
    // ------------------------------------------------------------------
    #[test]
    fn history_under_threshold_produces_no_mutations() {
        let w = worker(100);
        let history = users(50);
        assert!(evaluate(&w, history).is_empty());
    }

    // ------------------------------------------------------------------
    // 3. history_exactly_at_threshold_produces_no_mutations
    // ------------------------------------------------------------------
    #[test]
    fn history_exactly_at_threshold_produces_no_mutations() {
        let w = worker(100);
        let history = users(100);
        assert!(evaluate(&w, history).is_empty());
    }

    // ------------------------------------------------------------------
    // 4. history_one_over_threshold_prunes_oldest_pair
    //
    // Old tool pair (positions 0, 1) + 100 user entries = 102 entries.
    // Window = last 100 (positions 2..=101). Pair at 0,1 is in prune
    // region → both pruned (pair-atomic).
    // ------------------------------------------------------------------
    #[test]
    fn history_one_over_threshold_prunes_oldest_pair() {
        let w = worker(100);
        let mut history = Vec::new();

        // Old tool pair (positions 0, 1) — should be pruned.
        let p = bash_pair("tc-1", "ls", "out");
        history.push(p[0].clone());
        history.push(p[1].clone());
        let call_id = history[0].id.clone();
        let result_id = history[1].id.clone();

        // 100 user entries → total entries = 102. Window = last 100
        // (positions 2..=101). Pair at positions 0,1 is in the prune region.
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert_eq!(mutations.len(), 2, "exactly the pair should be pruned");
        assert!(excluded.contains(&call_id));
        assert!(excluded.contains(&result_id));
    }

    // ------------------------------------------------------------------
    // 5. pair_atomicity_when_result_straddles_cutoff
    //
    // Fixture: 100 user entries + ToolCall + 99 user entries + ToolResult
    // = 201 entries total. With max_age = 100, window starts at index
    // 201 - 100 = 101. The call at index 100 is in the prune region
    // (100 < 101); the result at index 200 is inside the keep window.
    // Both must be excluded (pair-atomic): the call is pruned because it's
    // in the prune region; the result is pruned because
    // `find_completed_matching_result` forwards from the call and finds
    // the matching result regardless of the keep-window boundary.
    // ------------------------------------------------------------------
    #[test]
    fn pair_atomicity_when_result_straddles_cutoff() {
        let w = worker(100);
        let mut history = Vec::new();

        // 100 filler user entries (positions 0..=99).
        history.extend(users(100));

        // The tool call at index 100 (in prune region once window is
        // computed: 100 < window_start = 101).
        let call = ChatEntry::tool_call("tc-straddle", "bash", r#"{"command": "ls"}"#);
        let call_id = call.id.clone();
        history.push(call);

        // 99 more user entries (positions 101..=199).
        history.extend(users(99));

        // The matching result at index 200 (inside keep window). Still
        // excluded because pair-atomicity pulls it in via the forward
        // scan from the call.
        let result =
            ChatEntry::tool_result("tc-straddle", "bash", "out", ToolResultStatus::Success);
        let result_id = result.id.clone();
        history.push(result);

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(
            excluded.contains(&call_id),
            "call must be excluded (in prune region)"
        );
        assert!(
            excluded.contains(&result_id),
            "result must be excluded (pair-atomic with pruned call)"
        );
    }

    // ------------------------------------------------------------------
    // 6. pair_at_keep_window_boundary_is_not_pruned
    //
    // Fixture: 100 user entries + call + result = 102 entries.
    // Window = last 100 → starts at index 102 - 100 = 2.
    // Call at index 100, result at index 101: both >= 2 → inside window
    // → not pruned. Loop runs `for i in 0..2`: examines only the first
    // two users, neither is a ToolCall, so no mutations are emitted.
    // ------------------------------------------------------------------
    #[test]
    fn pair_at_keep_window_boundary_is_not_pruned() {
        let w = worker(100);
        let mut history = Vec::new();

        // 100 user entries (positions 0..=99).
        history.extend(users(100));

        // Tool pair at positions 100, 101 — both inside keep window.
        let call = ChatEntry::tool_call("tc-boundary", "bash", r#"{"command": "ls"}"#);
        let call_id = call.id.clone();
        history.push(call);
        let result =
            ChatEntry::tool_result("tc-boundary", "bash", "out", ToolResultStatus::Success);
        let result_id = result.id.clone();
        history.push(result);

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(
            !excluded.contains(&call_id),
            "call inside keep window must not be pruned"
        );
        assert!(
            !excluded.contains(&result_id),
            "result inside keep window must not be pruned"
        );
        assert!(mutations.is_empty());
    }

    // ------------------------------------------------------------------
    // 7. pending_result_pair_is_skipped
    // ------------------------------------------------------------------
    #[test]
    fn pending_result_pair_is_skipped() {
        let w = worker(100);
        let mut history = Vec::new();

        // Old pending pair at positions 0, 1.
        let p = bash_pending_pair("tc-pending", "ls");
        history.push(p[0].clone());
        history.push(p[1].clone());

        // Push the window past the pair.
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        assert!(mutations.is_empty(), "pending pair must never be pruned");
    }

    // ------------------------------------------------------------------
    // 8. orphaned_call_with_no_result_is_skipped
    // ------------------------------------------------------------------
    #[test]
    fn orphaned_call_with_no_result_is_skipped() {
        let w = worker(100);
        let mut history = Vec::new();

        let call = ChatEntry::tool_call("tc-orphan", "bash", r#"{"command": "ls"}"#);
        history.push(call);

        history.extend(users(100));

        let mutations = evaluate(&w, history);
        assert!(mutations.is_empty(), "orphaned call must not be pruned");
    }

    // ------------------------------------------------------------------
    // 9. already_excluded_call_does_not_get_duplicate_mutation
    //
    // Old pair where the call is already ForcedExclude but the result is
    // not. Expect exactly 1 mutation (for the result only).
    // ------------------------------------------------------------------
    #[test]
    fn already_excluded_call_does_not_get_duplicate_mutation() {
        let w = worker(100);
        let mut history = Vec::new();

        let p = bash_pair("tc-1", "ls", "out");
        let mut call = p[0].clone();
        call.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() });
        let call_id = call.id.clone();
        history.push(call);
        let result = p[1].clone();
        let result_id = result.id.clone();
        history.push(result);

        history.extend(users(100));

        let mutations = evaluate(&w, history);
        assert_eq!(mutations.len(), 1, "only the non-excluded result mutates");
        let excluded = excluded_ids(&mutations);
        assert!(excluded.contains(&result_id));
        assert!(!excluded.contains(&call_id));
    }

    // ------------------------------------------------------------------
    // 9b. forced_included_call_does_not_get_mutation
    //
    // Same as test 9 but with ForcedInclude. The call is protected,
    // the result is not. Expect exactly 1 mutation (for the result only).
    // ------------------------------------------------------------------
    #[test]
    fn forced_included_call_does_not_get_mutation() {
        let w = worker(100);
        let mut history = Vec::new();

        let p = bash_pair("tc-1", "ls", "out");
        let mut call = p[0].clone();
        call.context_override = ContextOverride::ForcedInclude;
        let call_id = call.id.clone();
        history.push(call);
        let result = p[1].clone();
        let result_id = result.id.clone();
        history.push(result);

        history.extend(users(100));

        let mutations = evaluate(&w, history);
        assert_eq!(mutations.len(), 1, "only the non-protected result mutates");
        let excluded = excluded_ids(&mutations);
        assert!(excluded.contains(&result_id));
        assert!(!excluded.contains(&call_id));
    }

    // ------------------------------------------------------------------
    // 10. max_age_entries_clamped_to_1
    //
    // Config max_age = 0 (clamped to 1). Single complete pair = 2
    // entries. Window start = 2 - 1 = 1. Loop runs `for i in 0..1`,
    // examines the call. Forward scan finds the result at index 1.
    // Both halves excluded (pair-atomic).
    // ------------------------------------------------------------------
    #[test]
    fn max_age_entries_clamped_to_1() {
        let w = worker(0);
        let history: Vec<ChatEntry> = bash_pair("tc-clamp", "ls", "out").into();
        // 2 entries. max_age=0 → clamped to 1. Window starts at index 1
        // (the result). Loop runs `for i in 0..1`, examines the call.
        // find_completed_matching_result finds the result at index 1.
        // Both excluded.
        let mutations = evaluate(&w, history);
        assert_eq!(
            mutations.len(),
            2,
            "with max_age clamped to 1, the only pair is pruned"
        );
    }

    // ------------------------------------------------------------------
    // 11. multiple_tool_pairs_all_pruned_when_old
    //
    // 5 tool pairs scattered in the first 100 positions of a 200-entry
    // history. All 5 should be excluded (10 mutations).
    // ------------------------------------------------------------------
    #[test]
    fn multiple_tool_pairs_all_pruned_when_old() {
        let w = worker(100);
        let mut history = Vec::new();

        // 5 tool pairs (10 entries) at the start.
        for i in 0..5 {
            let p = bash_pair(&format!("tc-{i}"), "ls", "out");
            history.push(p[0].clone());
            history.push(p[1].clone());
        }

        // 190 user entries to push the total to 200.
        history.extend(users(190));

        let mutations = evaluate(&w, history);
        // 5 pairs * 2 mutations each = 10.
        assert_eq!(
            mutations.len(),
            10,
            "all 5 old pairs must be pruned, 2 mutations each"
        );
    }

    // ------------------------------------------------------------------
    // 12. non_tool_entries_in_prune_window_are_not_targeted
    //
    // Build history with old user/assistant entries plus one tool pair.
    // Only the tool pair should be excluded; user/assistant entries in
    // the prune region must NOT receive SetContextOverride mutations.
    // ------------------------------------------------------------------
    #[test]
    fn non_tool_entries_in_prune_window_are_not_targeted() {
        let w = worker(100);
        let mut history = Vec::new();

        // User and Assistant at the start.
        history.push(ChatEntry::user("old user"));
        let asst = ChatEntry::assistant("old assistant");
        let asst_id = asst.id.clone();
        history.push(asst);

        // Tool pair.
        let p = bash_pair("tc-mix", "ls", "out");
        history.push(p[0].clone());
        history.push(p[1].clone());

        // 100 user entries → total entries = 104, window covers last
        // 100 (positions 4..=103). Positions 0-3 (user, assistant, call,
        // result) are in the prune region. Only the pair (positions 2, 3)
        // should mutate.
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert_eq!(mutations.len(), 2);
        assert!(
            !excluded.contains(&asst_id),
            "non-tool assistant entry must not be pruned"
        );
    }
}
