//! Trivial-assistant auto-prune worker.
//!
//! Keeps the most recent `max_age_entries` (default: 100) in-context
//! entries' worth of history untouched, and prunes any `Assistant` entry
//! outside that window whose estimated token count is at most
//! `max_tokens` (default: 80). Targets low-value "narration" turns the
//! model emits between tool calls during autonomous coding.
//!
//! # Semantics
//!
//! - The threshold counts only entries where [`ChatEntry::is_in_context()`]
//!   returns `true`. Already-excluded, thinking, transient, system, error,
//!   and pending-result entries do not count.
//! - The window is measured from the end of history backward. Once
//!   `max_age_entries` in-context entries have been counted, everything
//!   older is in the prune window.
//! - Only `ChatEntryKind::Assistant(_)` entries are candidates. Non-assistant
//!   entries in the prune region are never targeted.
//! - Assistant entries inside the window are never pruned, regardless of
//!   token count.
//! - Assistant entries outside the window are pruned only if their
//!   estimated token count is `<= max_tokens`. Larger entries survive
//!   (a separate future worker will address large stale entries).
//! - Empty assistant entries are skipped defensively — they are already
//!   out of context via `is_empty_assistant()`, so they cannot be pruning
//!   candidates anyway.
//! - Already-`ForcedExclude` entries do not receive duplicate
//!   `SetContextOverride` mutations.
//! - Tokens are counted with the same `TiktokenCounter::o200k_base()`
//!   encoder used by the token-count actor and the UI minimap, so the
//!   `max_tokens` cutoff matches what users see.
//!
//! # Safety: pruning `Assistant` is unconditionally safe
//!
//! A `ToolCall` entry in `entries_to_messages` auto-creates an empty
//! `LlmMessage::Assistant { content: "", tool_calls: Some(vec![...]) }`
//! when its preceding `Assistant` message is missing. Excluding an
//! `Assistant` entry therefore cannot orphan a `ToolCall` or produce an
//! invalid provider request.
//!
//! # Example (max_age_entries = 4, max_tokens = 80, all entries in-context)
//!
//! ```text
//! X  [User]                  ← index 0 (untouched — not Assistant)
//!    [Assistant: "ok"]       ← index 1 (kept: inside window)
//! X  [Assistant: "done"]     ← index 2 (pruned: outside window AND ≤80 tokens)
//!    [Assistant: long...]    ← index 3 (kept: outside window but >80 tokens)
//!    [Tool Call]: bash       ← index 4 (kept: inside window; not a target)
//!    [Tool Result] (OK)      ← index 5 (kept)
//!    [Assistant: "ok"]       ← index 6 (kept: inside window)
//! ```

use std::sync::Arc;

use crate::feat::context::strategy::token_estimator::{TiktokenCounter, TokenCounter};
use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::TrivialAssistantAutoPruneConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

/// Trivial-assistant auto-prune worker.
///
/// See module docs for full semantics. Construct with
/// [`TrivialAssistantAutoPruneConfig`]; `max_age_entries` and `max_tokens`
/// are clamped to a minimum of 1 at evaluation time.
#[derive(Clone)]
pub struct TrivialAssistantAutoPruneWorker {
    /// Configuration for the trivial-assistant auto-prune strategy.
    pub config: TrivialAssistantAutoPruneConfig,
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
/// 2. For every `Assistant` entry at an index strictly less than the keep
///    window start, estimate its token count via `counter`.
/// 3. If the count is `<= max_tokens` AND the entry is not already
///    `ForcedExclude`, emit a `SetContextOverride::ForcedExclude` mutation.
fn build_trivial_assistant_mutations(
    history: &[ChatEntry],
    max_age: usize,
    max_tokens: usize,
    counter: &TiktokenCounter,
) -> Vec<HistoryMutation> {
    let max_age = max_age.max(1);
    let max_tokens = max_tokens.max(1);

    let Some(keep_window_start) = compute_keep_window_start(history, max_age) else {
        // Fewer than max_age in-context entries — nothing to prune.
        return Vec::new();
    };

    let mut mutations = Vec::new();

    for entry in history.iter().take(keep_window_start) {
        // Only Assistant entries in the prune region are candidates.
        let text = match &entry.kind {
            ChatEntryKind::Assistant(t) => t.as_str(),
            _ => continue,
        };

        // Defensive: empty assistant entries are already out of context
        // (is_empty_assistant() returns true → is_in_context() returns
        // false → they don't count toward the window and aren't eligible
        // for pruning). Skip explicitly to avoid emitting useless
        // mutations.
        if text.is_empty() {
            continue;
        }

        // Skip already-excluded entries to avoid duplicate mutations.
        if entry.context_override == ContextOverride::ForcedExclude {
            continue;
        }

        let tokens = counter.count(text);
        if tokens <= max_tokens {
            tracing::debug!(
                entry_id = %entry.id,
                tokens,
                max_tokens,
                keep_window_start,
                "trivial_assistant: excluding old trivial assistant entry"
            );
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: entry.id.clone(),
                value: ContextOverride::ForcedExclude,
            });
        }
    }

    mutations
}

#[async_trait::async_trait]
impl HistoryWorker for TrivialAssistantAutoPruneWorker {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "auto-prune-trivial-assistant"
    }

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        let counter = TiktokenCounter::o200k_base();
        let mutations = build_trivial_assistant_mutations(
            &history,
            self.config.max_age_entries,
            self.config.max_tokens,
            &counter,
        );
        tracing::debug!(
            mutations = mutations.len(),
            max_age_entries = self.config.max_age_entries,
            max_tokens = self.config.max_tokens,
            history_len = history.len(),
            "trivial_assistant evaluate done"
        );
        mutations
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::preferences_actor::user_preferences::TrivialAssistantAutoPruneConfig;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::protocol::SessionId;

    /// Build a worker with the given thresholds (enabled = true).
    fn worker(max_age: usize, max_tokens: usize) -> TrivialAssistantAutoPruneWorker {
        TrivialAssistantAutoPruneWorker {
            config: TrivialAssistantAutoPruneConfig {
                enabled: true,
                max_age_entries: max_age,
                max_tokens,
            },
        }
    }

    use crate::feat::session::chat_entry::ChatEntryId;

    /// Build N plain user entries (all in-context).
    fn users(n: usize) -> Vec<ChatEntry> {
        (0..n)
            .map(|i| ChatEntry::user(format!("user msg {i}")))
            .collect()
    }

    /// Build a trivial (≤80 tiktoken tokens) assistant entry.
    fn trivial_assistant(text: &str) -> ChatEntry {
        ChatEntry::assistant(text)
    }

    /// Build an assistant entry whose token count is reliably greater
    /// than 80 under the `o200k_base` encoding.
    fn large_assistant() -> ChatEntry {
        // ~500 chars of varied English prose → comfortably >80 o200k_base
        // tokens. Verified by the sanity assertion in
        // `large_assistant_outside_window_is_not_pruned`.
        let body = "This is a substantial assistant response with multiple sentences and \
             enough vocabulary to comfortably exceed eighty tiktoken tokens under \
             the o200k_base encoding used throughout the codebase. ";
        ChatEntry::assistant(body.repeat(4))
    }

    /// Evaluate the worker on a history snapshot.
    fn evaluate(
        w: &TrivialAssistantAutoPruneWorker,
        history: Vec<ChatEntry>,
    ) -> Vec<HistoryMutation> {
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
        let w = worker(100, 80);
        assert!(evaluate(&w, Vec::new()).is_empty());
    }

    // ------------------------------------------------------------------
    // 2. history_under_threshold_produces_no_mutations
    // ------------------------------------------------------------------
    #[test]
    fn history_under_threshold_produces_no_mutations() {
        let w = worker(100, 80);
        let mut history = users(50);
        history.insert(0, trivial_assistant("ok"));
        assert!(evaluate(&w, history).is_empty());
    }

    // ------------------------------------------------------------------
    // 3. history_exactly_at_threshold_produces_no_mutations
    //
    // 100 in-context entries (no overflow). Trivial assistant at idx 0
    // is inside the window → not pruned.
    // ------------------------------------------------------------------
    #[test]
    fn history_exactly_at_threshold_produces_no_mutations() {
        let w = worker(100, 80);
        let mut history = Vec::new();
        history.push(trivial_assistant("ok"));
        history.extend(users(99));
        assert_eq!(history.len(), 100);
        assert!(evaluate(&w, history).is_empty());
    }

    // ------------------------------------------------------------------
    // 4. trivial_assistant_outside_window_is_pruned
    // ------------------------------------------------------------------
    #[test]
    fn trivial_assistant_outside_window_is_pruned() {
        let w = worker(100, 80);
        let mut history = Vec::new();
        let asst = trivial_assistant("done");
        let asst_id = asst.id.clone();
        history.push(asst);
        // 100 user entries → total 101 in-context. Window covers last 100
        // (positions 1..=100). Position 0 (the assistant) is outside.
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert_eq!(mutations.len(), 1);
        assert!(excluded.contains(&asst_id));
    }

    // ------------------------------------------------------------------
    // 5. large_assistant_outside_window_is_not_pruned
    //
    // Sanity-check the test helper: ensure the "large" assistant text
    // actually exceeds 80 tiktoken tokens, then verify the worker leaves
    // it alone.
    // ------------------------------------------------------------------
    #[test]
    fn large_assistant_outside_window_is_not_pruned() {
        let counter = TiktokenCounter::o200k_base();
        let asst = large_assistant();
        let text = match &asst.kind {
            ChatEntryKind::Assistant(t) => t.clone(),
            _ => panic!("expected assistant"),
        };
        let tokens = counter.count(&text);
        assert!(
            tokens > 80,
            "test helper must produce >80 tokens, got {tokens}"
        );

        let w = worker(100, 80);
        let mut history = Vec::new();
        let asst = large_assistant();
        let asst_id = asst.id.clone();
        history.push(asst);
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(
            !excluded.contains(&asst_id),
            "large assistant outside window must NOT be pruned"
        );
        assert!(mutations.is_empty());
    }

    // ------------------------------------------------------------------
    // 6. trivial_assistant_inside_window_is_not_pruned
    //
    // Place the assistant AFTER 100 user entries so it is inside the
    // window.
    // ------------------------------------------------------------------
    #[test]
    fn trivial_assistant_inside_window_is_not_pruned() {
        let w = worker(100, 80);
        let mut history = users(100);
        let asst = trivial_assistant("ok");
        let asst_id = asst.id.clone();
        history.push(asst);
        // total = 101 in-context, window covers last 100 = idx 1..=100,
        // wait — 101 entries → window covers idx 1..=100? No, window is
        // the LAST 100 in-context entries. With 101 in-context, the
        // window starts at idx 1 (idx 0 is outside). The assistant at
        // idx 100 is inside the window.
        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(!excluded.contains(&asst_id));
        assert!(mutations.is_empty());
    }

    // ------------------------------------------------------------------
    // 7. empty_assistant_outside_window_is_not_targeted
    // ------------------------------------------------------------------
    #[test]
    fn empty_assistant_outside_window_is_not_targeted() {
        let w = worker(100, 80);
        let mut history = Vec::new();
        history.push(trivial_assistant(""));
        history.extend(users(100));
        let mutations = evaluate(&w, history);
        assert!(mutations.is_empty(), "empty assistant must not be targeted");
    }

    // ------------------------------------------------------------------
    // 8. non_assistant_entries_in_prune_window_are_not_targeted
    //
    // Old user/tool entries plus one trivial assistant. Only the
    // assistant should be pruned.
    // ------------------------------------------------------------------
    #[test]
    fn non_assistant_entries_in_prune_window_are_not_targeted() {
        let w = worker(100, 80);
        let mut history = Vec::new();
        let old_user = ChatEntry::user("old user");
        let old_user_id = old_user.id.clone();
        history.push(old_user);
        let asst = trivial_assistant("done");
        let asst_id = asst.id.clone();
        history.push(asst);
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert_eq!(mutations.len(), 1);
        assert!(excluded.contains(&asst_id));
        assert!(
            !excluded.contains(&old_user_id),
            "non-assistant entry must not be pruned"
        );
    }

    // ------------------------------------------------------------------
    // 9. already_excluded_assistant_does_not_get_duplicate_mutation
    // ------------------------------------------------------------------
    #[test]
    fn already_excluded_assistant_does_not_get_duplicate_mutation() {
        let w = worker(100, 80);
        let mut history = Vec::new();
        let mut asst = trivial_assistant("done");
        asst.context_override = ContextOverride::ForcedExclude;
        let asst_id = asst.id.clone();
        history.push(asst);
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        assert!(
            mutations.is_empty(),
            "already-excluded entry must not receive duplicate mutation"
        );
        // The assistant id should not appear in any mutation.
        let excluded = excluded_ids(&mutations);
        assert!(!excluded.contains(&asst_id));
    }

    // ------------------------------------------------------------------
    // 10. non_in_context_entries_do_not_count_toward_threshold
    //
    // 100 Thinking (not in-context) + 50 in-context (incl 1 assistant at
    // the start). Only 50 in-context → below threshold of 100 → no prune.
    // ------------------------------------------------------------------
    #[test]
    fn non_in_context_entries_do_not_count_toward_threshold() {
        let w = worker(100, 80);
        let mut history = Vec::new();
        for i in 0..100 {
            history.push(ChatEntry::thinking(format!("thought {i}")));
        }
        history.push(trivial_assistant("ok"));
        history.extend(users(49));

        let mutations = evaluate(&w, history);
        assert!(
            mutations.is_empty(),
            "with only 50 in-context entries (below threshold), nothing prunes"
        );
    }

    // ------------------------------------------------------------------
    // 11. forced_include_entries_count_toward_threshold
    //
    // Trivial assistant + ForcedInclude entry + 99 users = 101 in-context.
    // Window covers last 100 → trivial assistant (idx 0) pruned. This
    // confirms ForcedInclude entries count toward the threshold (otherwise
    // total in-context would be 100 and nothing would prune).
    // ------------------------------------------------------------------
    #[test]
    fn forced_include_entries_count_toward_threshold() {
        let w = worker(100, 80);
        let mut history = Vec::new();
        let mut fi = ChatEntry::user("forced");
        fi.context_override = ContextOverride::ForcedInclude;
        history.push(fi);
        let asst = trivial_assistant("done");
        let asst_id = asst.id.clone();
        history.push(asst);
        let mut fi = ChatEntry::user("forced");
        fi.context_override = ContextOverride::ForcedInclude;
        history.push(fi);
        history.extend(users(99));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(excluded.contains(&asst_id));
    }

    // ------------------------------------------------------------------
    // 12. max_age_entries_clamped_to_1
    //
    // max_age = 0 → clamped to 1. Two in-context entries (user, trivial
    // assistant). Window covers only the last entry (user). The trivial
    // assistant at idx 0 is outside the (clamped) window → pruned.
    // ------------------------------------------------------------------
    #[test]
    fn max_age_entries_clamped_to_1() {
        let w = worker(0, 80);
        let mut history = Vec::new();
        let asst = trivial_assistant("done");
        let asst_id = asst.id.clone();
        history.push(asst);
        history.push(ChatEntry::user("after"));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(excluded.contains(&asst_id));
    }

    // ------------------------------------------------------------------
    // 13. max_tokens_clamped_to_1
    //
    // max_tokens = 0 → clamped to 1. Any non-empty assistant counts as
    // ≤1 token? No — clamping to 1 means "prune if tokens <= 1". A
    // single-word assistant like "ok" is exactly 1 o200k_base token.
    // ------------------------------------------------------------------
    #[test]
    fn max_tokens_clamped_to_1() {
        let counter = TiktokenCounter::o200k_base();
        let text = "ok";
        let tokens = counter.count(text);
        assert_eq!(tokens, 1, "test assumes 'ok' is 1 token under o200k_base");

        let w = worker(100, 0);
        let mut history = Vec::new();
        let asst = trivial_assistant(text);
        let asst_id = asst.id.clone();
        history.push(asst);
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(
            excluded.contains(&asst_id),
            "with max_tokens clamped to 1, 'ok' (1 token) is pruned"
        );
    }

    // ------------------------------------------------------------------
    // 14. multiple_trivial_assistants_all_pruned_when_old
    //
    // 5 trivial assistants scattered in the first 100 positions of a
    // 200-entry history. All 5 should be excluded.
    // ------------------------------------------------------------------
    #[test]
    fn multiple_trivial_assistants_all_pruned_when_old() {
        let w = worker(100, 80);
        let mut history = Vec::new();
        let mut expected_ids = Vec::new();
        for i in 0..5 {
            let asst = trivial_assistant(&format!("narration {i}"));
            expected_ids.push(asst.id.clone());
            history.push(asst);
        }
        // 195 user entries → total 200 in-context. Window covers last 100.
        history.extend(users(195));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert_eq!(mutations.len(), 5);
        for id in &expected_ids {
            assert!(excluded.contains(id), "expected {id} to be pruned");
        }
    }
}
