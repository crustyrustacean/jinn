//! Tool-age-window auto-prune worker.
//!
//! Keeps only the most recent `max_age_entries` (default: 100) in-context
//! entries' worth of tool activity. Any `ToolCall`/`ToolResult` pair older
//! than that window is marked [`ForcedExclude`] so the LLM never sees stale
//! tool output.
//!
//! # Semantics
//!
//! - The threshold counts only entries where
//!   [`ChatEntry::is_in_context()`] returns `true`. Already-excluded,
//!   thinking, transient, system, error, and pending-result entries do not
//!   count.
//! - The window is measured from the end of history backward. Once
//!   `max_age_entries` in-context entries have been counted, everything
//!   older is in the prune window.
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
//! # Example (max_age_entries = 4, all entries in-context)
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
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride};
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
/// Walks backward from the end of history, counting entries where
/// [`ChatEntry::is_in_context()`] is `true`. Once `max_age` in-context
/// entries have been counted, the index where the count reached `max_age`
/// is the **first** in-context entry inside the keep window.
///
/// Returns `None` if fewer than `max_age` in-context entries exist in
/// history (nothing to prune).
fn compute_keep_window_start(history: &[ChatEntry], max_age: usize) -> Option<usize> {
    let mut counted = 0usize;
    for i in (0..history.len()).rev() {
        if history[i].is_in_context() {
            counted += 1;
            if counted == max_age {
                return Some(i);
            }
        }
    }
    None
}

/// Build the list of `SetContextOverride::ForcedExclude` mutations for a
/// single snapshot.
///
/// Pure function (no `&self`) so unit tests can call it directly without
/// spinning up a tokio runtime.
///
/// Algorithm:
/// 1. Find the keep window start index (first in-context entry inside the
///    window).
/// 2. For every `ToolCall` at an index strictly less than the keep window
///    start, attempt to find its completed matching result.
/// 3. If found, emit `ForcedExclude` mutations for both halves — unless an
///    individual half is already excluded.
///
/// Pair-atomicity across the cutoff is preserved by the forward scan in
/// `find_completed_matching_result`: even if the result lives at an index
/// `>= keep_window_start`, it is still found and excluded together with
/// its call.
fn build_age_window_mutations(history: &[ChatEntry], max_age: usize) -> Vec<HistoryMutation> {
    let max_age = max_age.max(1);

    let Some(keep_window_start) = compute_keep_window_start(history, max_age) else {
        // Fewer than max_age in-context entries — nothing to prune.
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
        let call_already_excluded = entry.context_override == ContextOverride::ForcedExclude;

        // Find the matching non-pending result. If none (orphaned or still
        // pending), skip the entire pair.
        let Some(result_id) = find_completed_matching_result(history, i, &tool_call_id) else {
            continue;
        };

        // Locate the result entry to check its exclude state. Forward scan
        // from i+1 — guaranteed to find it because find_completed_matching_result
        // just did.
        let result_already_excluded = history
            .iter()
            .skip(i + 1)
            .find(|e| e.id == result_id)
            .is_some_and(|e| e.context_override == ContextOverride::ForcedExclude);

        // Emit mutations only for halves not already excluded. Pair-atomicity
        // is preserved at the *decision* level (we always consider both
        // halves), even if one half is already excluded and only one
        // mutation is emitted.
        if !call_already_excluded {
            tracing::debug!(
                entry_id = %call_id,
                keep_window_start,
                "tool_age_window: excluding old tool call"
            );
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: call_id,
                value: ContextOverride::ForcedExclude,
            });
        }
        if !result_already_excluded {
            tracing::debug!(
                entry_id = %result_id,
                keep_window_start,
                "tool_age_window: excluding old tool result"
            );
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: result_id,
                value: ContextOverride::ForcedExclude,
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
        let mutations = build_age_window_mutations(&history, self.config.max_age_entries);
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

        // 100 user entries — pushes total in-context count to 102, so the
        // pair at positions 0,1 falls outside the window of the most recent
        // 100 in-context entries.
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert_eq!(mutations.len(), 2, "exactly the pair should be pruned");
        assert!(excluded.contains(&call_id));
        assert!(excluded.contains(&result_id));
    }

    // ------------------------------------------------------------------
    // 5. pair_atomicity_when_result_straddles_cutoff
    // ------------------------------------------------------------------
    //
    // Scenario: history of 100 in-context entries, then a tool pair at the
    // tail end. With max_age = 100:
    //   - keep window starts at the call's position (call is the 100th
    //     in-context entry from the end), so call is at keep_window_start
    //     and is NOT pruned (loop is `for i in 0..keep_window_start`).
    //
    // To exercise straddling, we need the call INSIDE the prune region and
    // the result OUTSIDE. Construct: 100 user entries, then ToolCall, then
    // 99 user entries, then ToolResult. With max_age = 100, the keep
    // window's 100 in-context entries are: result + 99 users. The call
    // sits before them, in the prune region. The result is inside the
    // keep window. Both must be excluded (pair-atomic).
    #[test]
    fn pair_atomicity_when_result_straddles_cutoff() {
        let w = worker(100);
        let mut history = Vec::new();

        // 100 filler user entries.
        history.extend(users(100));

        // The tool call (in prune region once window is computed).
        let call = ChatEntry::tool_call("tc-straddle", "bash", r#"{"command": "ls"}"#);
        let call_id = call.id.clone();
        history.push(call);

        // 99 more user entries.
        history.extend(users(99));

        // The matching result (inside keep window).
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
    // 6. pair_atomicity_when_both_sides_in_keep_window
    //
    // If both call and result are inside the keep window, neither is
    // pruned. We also check the case where the call is the entry that
    // brings the in-context count to max_age (call at keep_window_start,
    // result just after) — neither should be pruned.
    // ------------------------------------------------------------------
    #[test]
    fn pair_at_keep_window_boundary_is_not_pruned() {
        let w = worker(100);
        let mut history = Vec::new();

        // 100 user entries (the entire keep window).
        history.extend(users(100));

        // Tool pair at positions 100, 101 — both outside the prune region
        // (window covers positions 0..=100). Actually with 100 in-context
        // users + call (in-context) + result (in-context), the 100 most
        // recent are positions 2..=101. Window starts at index 2.
        let call = ChatEntry::tool_call("tc-boundary", "bash", r#"{"command": "ls"}"#);
        let call_id = call.id.clone();
        history.push(call);
        let result =
            ChatEntry::tool_result("tc-boundary", "bash", "out", ToolResultStatus::Success);
        let result_id = result.id.clone();
        history.push(result);

        // History in-context count = 102. Window start = index 2.
        // Entries 0 and 1 (two users) are in prune region but aren't
        // ToolCalls, so no mutations. The pair is entirely in the keep
        // window.
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
    // 9. already_excluded_entries_do_not_count_toward_threshold
    //
    // 200 entries, first 150 ForcedExclude. Last 50 in-context.
    // max_age = 100. Only 50 in-context entries exist → no mutations.
    // ------------------------------------------------------------------
    #[test]
    fn already_excluded_entries_do_not_count_toward_threshold() {
        let w = worker(100);
        let mut history = Vec::new();

        // 150 excluded user entries.
        for i in 0..150 {
            let mut e = ChatEntry::user(format!("excluded {i}"));
            e.context_override = ContextOverride::ForcedExclude;
            history.push(e);
        }

        // 50 in-context entries.
        history.extend(users(50));

        // Total in-context = 50, less than max_age 100.
        assert!(evaluate(&w, history).is_empty());
    }

    // ------------------------------------------------------------------
    // 10. already_excluded_entries_do_not_receive_duplicate_mutations
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
        call.context_override = ContextOverride::ForcedExclude;
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
    // 11. thinking_transient_system_entries_do_not_count
    //
    // 150 entries: 100 Thinking (not in-context) + 50 in-context.
    // max_age = 100. Only 50 in-context → no mutations.
    // ------------------------------------------------------------------
    #[test]
    fn non_in_context_entry_types_do_not_count() {
        let w = worker(100);
        let mut history = Vec::new();

        // Thinking entries are excluded by default.
        for i in 0..100 {
            history.push(ChatEntry::thinking(format!("thought {i}")));
        }

        // 50 in-context user entries.
        history.extend(users(50));

        assert!(evaluate(&w, history).is_empty());
    }

    // ------------------------------------------------------------------
    // 12. forced_include_entries_count
    //
    // An entry with ForcedInclude should count toward the threshold.
    // Build: 1 ForcedInclude user, then 99 default users, then tool pair
    // at end. Total in-context = 102, window covers most recent 100.
    // Pair is inside window → not pruned. (Tests that ForcedInclude
    // entries are counted.)
    //
    // Counter-test: 1 ForcedInclude + 99 users + tool pair (call+result)
    // = 102 in-context entries. Window is last 100. The pair (positions
    // 100, 101) is inside the window. The two oldest users (positions 0,
    // 1) are in the prune region but not tool calls. → no mutations.
    //
    // Now flip: put the pair FIRST. 1 ForcedInclude + pair (2) + 97 users
    // = 100 in-context entries. With max_age = 100, that's exactly at
    // threshold → no prune. Add one more user: 101 in-context. The
    // ForcedInclude entry (position 0) is the oldest, pair (positions 1,
    // 2) is in the prune region.
    // ------------------------------------------------------------------
    #[test]
    fn forced_include_entries_count_toward_threshold() {
        let w = worker(100);
        let mut history = Vec::new();

        // ForcedInclude entry (counts as in-context).
        let mut fi = ChatEntry::user("forced include");
        fi.context_override = ContextOverride::ForcedInclude;
        history.push(fi);

        // Old pair.
        let p = bash_pair("tc-fi", "ls", "out");
        history.push(p[0].clone());
        history.push(p[1].clone());
        let call_id = history[1].id.clone();
        let result_id = history[2].id.clone();

        // 99 more user entries → total in-context = 1 + 2 + 99 = 102.
        // The pair is at idx 1,2; cutoff_idx (100th from end) = 2;
        // so idx 1 (call) is below cutoff and eligible.
        history.extend(users(99));
        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(
            excluded.contains(&call_id),
            "old pair's call must be pruned"
        );
        assert!(
            excluded.contains(&result_id),
            "old pair's result must be pruned"
        );
    }

    // ------------------------------------------------------------------
    // 13. max_age_entries_clamped_to_1
    //
    // Config max_age = 0. History with 2 in-context entries: user, tool
    // call. The most recent in-context entry is the tool call itself —
    // but a ToolCall is not a complete pair (no result), so no mutation.
    // Use a complete pair instead: call + result, total 2 in-context.
    // max_age clamped to 1: keep window is just the result. The call is
    // in the prune region → both pruned (pair-atomic).
    // ------------------------------------------------------------------
    #[test]
    fn max_age_entries_clamped_to_1() {
        let w = worker(0);
        let history: Vec<ChatEntry> = bash_pair("tc-clamp", "ls", "out").into();
        // 2 in-context entries. max_age=0 → clamped to 1. Window starts
        // at index 1 (the result). Loop runs `for i in 0..1`, examines
        // the call. find_completed_matching_result finds the result at
        // index 1. Both excluded.
        let mutations = evaluate(&w, history);
        assert_eq!(
            mutations.len(),
            2,
            "with max_age clamped to 1, the only pair is pruned"
        );
    }

    // ------------------------------------------------------------------
    // 14. multiple_tool_pairs_all_pruned_when_old
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
    // 15. non_tool_entries_in_prune_window_are_not_targeted
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

        // 100 user entries → total in-context = 104, window covers last
        // 100. Positions 0-3 (user, assistant, call, result) are in the
        // prune region. Only the pair (positions 2, 3) should mutate.
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
