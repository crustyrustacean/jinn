//! User-anchor-radius auto-prune worker.
//!
//! Targets large (`> 80` token) `Assistant` text entries that are far from
//! any `User` entry in either direction. These are typically planning
//! narration, status updates, or mid-chain commentary the model emits to
//! itself during autonomous coding — entries that the
//! [`TrivialAssistantAutoPruneWorker`] leaves alone (it only handles
//! `<= 80` tokens).
//!
//! # Semantics
//!
//! For each `Assistant` entry:
//! 1. Skip if the entry is empty (placeholder for tool-call-only responses).
//! 2. Skip if already [`ForcedExclude`]d.
//! 3. Skip if the entry is pinned.
//! 4. Look up (or compute and cache) the token count via the shared
//!    [`HistoryWorkerChatEntryTokenCache`].
//! 5. Skip if `tokens <= 80` (owned by `TrivialAssistantAutoPruneWorker`).
//! 6. Find the nearest preceding `User` entry's index distance (`d_back`).
//!    `∞` if no preceding user.
//! 7. Find the nearest following `User` entry's index distance (`d_fwd`).
//!    `∞` if no following user.
//! 8. If **both** anchors are absent (no `User` anywhere), skip the entry
//!    entirely — emit no mutation. See [`build_prune_mutations`] docs for
//!    the rationale.
//! 9. Otherwise, prune if `min(d_back, d_fwd) > radius`.
//!
//! Distance is measured in **raw chat entries** (index diff), independent of
//! any other worker's [`ForcedExclude`] decisions, so multiple workers
//! compose cleanly.
//!
//! # Token-cache integration
//!
//! Per-entry counts are looked up via
//! [`HistoryWorkerChatEntryTokenCache::get_or_insert_with`]. The first
//! worker to evaluate a session pays the tiktoken cost; subsequent
//! evaluations and concurrent workers hit the cache.
//!
//! # Safety: pruning `Assistant` is unconditionally safe
//!
//! A `ToolCall` entry in `entries_to_messages` auto-creates an empty
//! `LlmMessage::Assistant { content: "", tool_calls: Some(vec![...]) }`
//! when its preceding `Assistant` message is missing. Excluding an
//! `Assistant` entry therefore cannot orphan a `ToolCall` or produce an
//! invalid provider request.
//!
//! # Example (`radius = 4`, all assistant entries `> 80` tokens)
//!
//! ```text
//!    [User]                  ← index 0  (anchor)
//!    [Assistant: long...]    ← index 1  (kept: d_back=1 ≤ R)
//!    [Tool Call]: bash       ← index 2  (not a target)
//!    [Tool Result]           ← index 3  (not a target)
//! X  [Assistant: long...]    ← index 4  (pruned: d_back=4, d_fwd=∞, min=∞ > R=4)
//! X  [Assistant: long...]    ← index 5  (pruned: d_back=5, d_fwd=∞, min=∞ > R=4)
//!    [User]                  ← index 6  (anchor)
//!    [Assistant: long...]    ← index 7  (kept: d_back=1 ≤ R)
//!    [Assistant: long...]    ← index 8  (kept: d_fwd=2 ≤ R; wrap-up summary)
//! ```
//!
//! [`ForcedExclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedExclude
//! [`TrivialAssistantAutoPruneWorker`]: crate::feat::auto_prune_worker::TrivialAssistantAutoPruneWorker
//! [`HistoryWorkerChatEntryTokenCache`]: crate::feat::auto_prune_worker::HistoryWorkerChatEntryTokenCache

use std::sync::Arc;

use crate::feat::context::strategy::token_estimator::{TiktokenCounter, TokenCounter};
use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::UserAnchorRadiusAutoPruneConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

/// Minimum token count for an `Assistant` entry to be a prune candidate.
///
/// Entries at or below this threshold are owned by
/// [`TrivialAssistantAutoPruneWorker`](super::TrivialAssistantAutoPruneWorker).
const MIN_CANDIDATE_TOKENS: u32 = 81;

/// User-anchor-radius auto-prune worker.
///
/// See the [module docs](self) for full semantics.
#[derive(Clone)]
pub struct UserAnchorRadiusAutoPruneWorker {
    /// Configuration for the user-anchor-radius strategy.
    pub config: UserAnchorRadiusAutoPruneConfig,
    /// Shared per-session, per-entry token-count cache. Cheap clone (inner is
    /// `Arc`-shared).
    pub token_cache: super::HistoryWorkerChatEntryTokenCache,
    /// Long-lived tiktoken counter. Cheap copy (`Copy` type with `&'static`
    /// encoder reference).
    pub counter: TiktokenCounter,
}

/// Collect the indices of every `User` entry in history, in order.
///
/// Returns an empty `Vec` when there are no `User` entries (the caller
/// treats this as "no anchor" and emits no mutations).
fn collect_user_indices(history: &[ChatEntry]) -> Vec<usize> {
    history
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e.kind {
            ChatEntryKind::User { .. } => Some(i),
            _ => None,
        })
        .collect()
}

/// Compute `(d_back, d_fwd)` — index distances to the nearest preceding and
/// following `User` entries.
///
/// `user_indices` must be sorted ascending (the natural output of
/// [`collect_user_indices`]). `idx` is the position of the candidate
/// `Assistant` entry.
///
/// Returns `(None, None)` when there are no user entries at all.
/// Otherwise returns distances where `None` means "no anchor on that side"
/// (i.e., distance is `∞`).
fn distances_to_nearest_users(
    idx: usize,
    user_indices: &[usize],
) -> (Option<usize>, Option<usize>) {
    if user_indices.is_empty() {
        return (None, None);
    }

    // Binary search: Ok gives exact match (impossible: a user index cannot
    // equal an assistant index in our callers). Err gives the insertion
    // point - the count of user indices strictly less than `idx`.
    let Err(insertion) = user_indices.binary_search(&idx) else {
        return (None, None); // defensive: treat as no anchor
    };

    // Preceding user = user_indices[insertion - 1] if insertion > 0.
    let d_back = insertion.checked_sub(1).map(|i| idx - user_indices[i]);
    // Following user = user_indices[insertion] if insertion < len.
    let d_fwd = user_indices
        .get(insertion)
        .map(|&user_idx| user_idx - idx);

    (d_back, d_fwd)
}

/// Build the list of `SetContextOverride::ForcedExclude` mutations for a
/// single snapshot.
///
/// Pure function (no `&self`) so unit tests can call it directly without
/// spinning up a tokio runtime.
///
/// # Algorithm
///
/// 1. Pre-compute `user_indices` via [`collect_user_indices`].
/// 2. If `user_indices` is empty, return `Vec::new()` — no anchor exists
///    anywhere, so we make no decisions (per spec: "no-anchor = skip").
/// 3. For each `Assistant` entry:
///    a. Skip empty, pinned, or already-excluded entries.
///    b. Look up or compute token count via the shared cache.
///    c. Skip if `tokens <= 80`.
///    d. Compute `(d_back, d_fwd)` via [`distances_to_nearest_users`].
///    e. Prune if both distances are present and both strictly exceed
///    `radius`.
///    f. Skip otherwise (including the case where only one side is
///    missing - that side acts as `∞`, but the present side is finite
///    and we keep the entry if it's within radius).
fn build_prune_mutations(
    history: &[ChatEntry],
    radius: usize,
    session_id: &SessionId,
    token_cache: &super::HistoryWorkerChatEntryTokenCache,
    counter: &TiktokenCounter,
) -> Vec<HistoryMutation> {
    let radius = radius.max(1);

    let user_indices = collect_user_indices(history);
    if user_indices.is_empty() {
        return Vec::new();
    }

    let mut mutations = Vec::new();

    for (idx, entry) in history.iter().enumerate() {
        // Only Assistant entries are candidates.
        let text = match &entry.kind {
            ChatEntryKind::Assistant(t) => t.as_str(),
            _ => continue,
        };

        // Empty assistant entries are placeholders for tool-call-only
        // responses. Skip defensively.
        if text.is_empty() {
            continue;
        }

        // Skip pinned entries — pin beats ForcedExclude.
        if entry.is_pinned() {
            continue;
        }

        // Skip already-excluded entries — idempotency.
        if entry.context_override == ContextOverride::ForcedExclude {
            continue;
        }

        // Look up or compute token count. The closure only fires on first
        // miss for this (session, entry) pair.
        let tokens = token_cache.get_or_insert_with(session_id, &entry.id, || {
            counter.count(text) as u32
        });

        // Skip small entries — owned by TrivialAssistantAutoPruneWorker.
        if tokens < MIN_CANDIDATE_TOKENS {
            continue;
        }

        let (d_back, d_fwd) = distances_to_nearest_users(idx, &user_indices);

        // Prune if both sides have anchors and both exceed radius.
        let prune = match (d_back, d_fwd) {
            (Some(db), Some(df)) => db > radius && df > radius,
            // One side has no anchor (distance = ∞). The other side is
            // finite; keep if within radius, prune if outside.
            (Some(db), None) => db > radius,
            (None, Some(df)) => df > radius,
            // No anchors at all — already ruled out by the empty-check
            // above, but defensive: skip.
            (None, None) => continue,
        };

        if prune {
            tracing::debug!(
                entry_id = %entry.id,
                tokens,
                radius,
                d_back = ?d_back,
                d_fwd = ?d_fwd,
                "user_anchor_radius: excluding stale large assistant entry"
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
impl HistoryWorker for UserAnchorRadiusAutoPruneWorker {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "auto-prune-user-anchor-radius"
    }

    async fn evaluate(
        &self,
        session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        let mutations = build_prune_mutations(
            &history,
            self.config.radius,
            session_id,
            &self.token_cache,
            &self.counter,
        );
        tracing::debug!(
            mutations = mutations.len(),
            radius = self.config.radius,
            history_len = history.len(),
            "user_anchor_radius evaluate done"
        );
        mutations
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::preferences_actor::user_preferences::UserAnchorRadiusAutoPruneConfig;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::chat_entry::ChatEntryId;
    use crate::protocol::SessionId;

    // ------------------------------------------------------------------
    // Test helpers
    // ------------------------------------------------------------------

    /// Build a worker with the given radius (enabled = true).
    fn worker(radius: usize) -> UserAnchorRadiusAutoPruneWorker {
        UserAnchorRadiusAutoPruneWorker {
            config: UserAnchorRadiusAutoPruneConfig {
                enabled: true,
                radius,
            },
            token_cache: super::super::HistoryWorkerChatEntryTokenCache::new(),
            counter: TiktokenCounter::o200k_base(),
        }
    }

    /// Build N plain user entries.
    fn users(n: usize) -> Vec<ChatEntry> {
        (0..n)
            .map(|i| ChatEntry::user(format!("user msg {i}")))
            .collect()
    }

    /// Build a trivial (≤80 tiktoken tokens) assistant entry.
    fn trivial_assistant(text: &str) -> ChatEntry {
        ChatEntry::assistant(text)
    }

    /// Build an assistant entry whose token count is reliably greater than
    /// 80 under the `o200k_base` encoding.
    fn large_assistant() -> ChatEntry {
        // ~500 chars of varied English prose → comfortably >80 o200k_base
        // tokens. Mirrors the helper in trivial_assistant.rs.
        let body = "This is a substantial assistant response with multiple sentences and \
             enough vocabulary to comfortably exceed eighty tiktoken tokens under \
             the o200k_base encoding used throughout the codebase. ";
        ChatEntry::assistant(body.repeat(4))
    }

    /// Evaluate the worker on a history snapshot.
    fn evaluate(
        w: &UserAnchorRadiusAutoPruneWorker,
        history: Vec<ChatEntry>,
    ) -> Vec<HistoryMutation> {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let history: Arc<[ChatEntry]> = history.into();
        rt.block_on(async { w.evaluate(&SessionId::new(), history).await })
    }

    /// Collect entry ids targeted by `SetContextOverride::ForcedExclude`.
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
    // 2. no_user_entries_produces_no_mutations
    //
    // Even with large assistant entries, no User anchor → skip all.
    // ------------------------------------------------------------------
    #[test]
    fn no_user_entries_produces_no_mutations() {
        let w = worker(100);
        let history = vec![large_assistant(), large_assistant(), large_assistant()];
        assert!(
            evaluate(&w, history).is_empty(),
            "no User entries → no mutations"
        );
    }

    // ------------------------------------------------------------------
    // 3. only_assistant_no_user_produces_no_mutations
    // ------------------------------------------------------------------
    #[test]
    fn only_assistant_no_user_produces_no_mutations() {
        let w = worker(100);
        let mut history = vec![large_assistant()];
        history.extend(std::iter::repeat_n(trivial_assistant("ok"), 50));
        history.push(large_assistant());
        assert!(evaluate(&w, history).is_empty());
    }

    // ------------------------------------------------------------------
    // 4. small_assistant_near_user_is_not_pruned
    //
    // Trivial assistant just after a User — skipped because tokens <= 80.
    // ------------------------------------------------------------------
    #[test]
    fn small_assistant_near_user_is_not_pruned() {
        let w = worker(2);
        let mut history = vec![ChatEntry::user("hi")];
        let asst = trivial_assistant("ok");
        let asst_id = asst.id.clone();
        history.push(asst);
        history.extend(users(100));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(!excluded.contains(&asst_id));
        assert!(mutations.is_empty());
    }

    // ------------------------------------------------------------------
    // 5. large_assistant_far_from_user_is_pruned
    //
    // User at index 0, large assistants at indices 1 and 102 (distance
    // from user 1 and 102), then 100 users to push the second large
    // assistant far from any user. With radius=50: index 1 (d=1) kept,
    // index 102 (d=102 from user, d_fwd=∞) pruned.
    // ------------------------------------------------------------------
    #[test]
    fn large_assistant_far_from_user_is_pruned() {
        let w = worker(50);
        let mut history = vec![ChatEntry::user("start")];
        let kept = large_assistant();
        let kept_id = kept.id.clone();
        history.push(kept);
        // 100 trivial assistants — they're filtered by tokens, but they
        // push the next large assistant far from any user entry.
        history.extend(std::iter::repeat_n(trivial_assistant("step"), 100));
        let pruned = large_assistant();
        let pruned_id = pruned.id.clone();
        history.push(pruned);

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(excluded.contains(&pruned_id), "distant large assistant must be pruned");
        assert!(!excluded.contains(&kept_id), "nearby large assistant must be kept");
    }

    // ------------------------------------------------------------------
    // 6. radius_boundary_at_exact_radius_is_kept
    //
    // User at 0, large assistant at index R. Distance = R. The rule is
    // "strictly greater than R" → kept.
    // ------------------------------------------------------------------
    #[test]
    fn radius_boundary_at_exact_radius_is_kept() {
        let radius = 10;
        let w = worker(radius);
        let mut history = vec![ChatEntry::user("anchor")];
        // Fill indices 1..radius with trivial assistants (small, skipped).
        history.extend(std::iter::repeat_n(trivial_assistant("x"), radius - 1));
        // Place the large assistant at index `radius`.
        let asst = large_assistant();
        let asst_id = asst.id.clone();
        history.push(asst);

        let mutations = evaluate(&w, history);
        assert!(
            !excluded_ids(&mutations).contains(&asst_id),
            "assistant at d_back = R must be kept"
        );
    }

    // ------------------------------------------------------------------
    // 7. radius_boundary_at_radius_plus_one_is_pruned
    //
    // Same setup as #6 but large assistant at index R+1.
    // ------------------------------------------------------------------
    #[test]
    fn radius_boundary_at_radius_plus_one_is_pruned() {
        let radius = 10;
        let w = worker(radius);
        let mut history = vec![ChatEntry::user("anchor")];
        history.extend(std::iter::repeat_n(trivial_assistant("x"), radius));
        let asst = large_assistant();
        let asst_id = asst.id.clone();
        history.push(asst);

        let mutations = evaluate(&w, history);
        assert!(
            excluded_ids(&mutations).contains(&asst_id),
            "assistant at d_back = R+1 must be pruned"
        );
    }

    // ------------------------------------------------------------------
    // 8. forward_anchor_protects_wrap_up
    //
    // User at index 0, large assistant at index 0+radius+1 (would be
    // pruned backward-only), but another user follows shortly after.
    // ------------------------------------------------------------------
    #[test]
    fn forward_anchor_protects_wrap_up() {
        let radius = 5;
        let w = worker(radius);
        let mut history = vec![ChatEntry::user("start")];
        // Push the large assistant out beyond backward radius.
        history.extend(std::iter::repeat_n(trivial_assistant("x"), radius + 5));
        let asst = large_assistant();
        let asst_id = asst.id.clone();
        history.push(asst);
        // Now add a user 1 entry later — the large assistant is the
        // "I'm done" wrap-up just before the next user.
        history.push(ChatEntry::user("next"));

        let mutations = evaluate(&w, history);
        assert!(
            !excluded_ids(&mutations).contains(&asst_id),
            "wrap-up assistant near forward user must be kept"
        );
    }

    // ------------------------------------------------------------------
    // 9. no_preceding_user_only_following_user_within_radius_kept
    //
    // Large assistant at index 0 (no preceding user), user at index 3.
    // d_fwd = 3 ≤ radius → kept.
    // ------------------------------------------------------------------
    #[test]
    fn no_preceding_user_only_following_user_within_radius_kept() {
        let w = worker(5);
        let asst = large_assistant();
        let asst_id = asst.id.clone();
        let mut history = vec![asst];
        // Fill with trivial entries.
        history.push(trivial_assistant("a"));
        history.push(trivial_assistant("b"));
        // User at index 3.
        history.push(ChatEntry::user("anchor"));

        let mutations = evaluate(&w, history);
        assert!(
            !excluded_ids(&mutations).contains(&asst_id),
            "assistant within forward radius must be kept"
        );
    }

    // ------------------------------------------------------------------
    // 10. no_preceding_user_only_following_user_beyond_radius_pruned
    //
    // Large assistant at index 0, user at index radius+2.
    // d_fwd = radius+2 > R → pruned.
    // ------------------------------------------------------------------
    #[test]
    fn no_preceding_user_only_following_user_beyond_radius_pruned() {
        let radius = 3;
        let w = worker(radius);
        let asst = large_assistant();
        let asst_id = asst.id.clone();
        let mut history = vec![asst];
        // Fill with trivial entries so the user is far.
        history.extend(std::iter::repeat_n(trivial_assistant("x"), radius + 1));
        history.push(ChatEntry::user("far anchor"));

        let mutations = evaluate(&w, history);
        assert!(
            excluded_ids(&mutations).contains(&asst_id),
            "assistant beyond forward radius must be pruned"
        );
    }

    // ------------------------------------------------------------------
    // 11. already_excluded_entry_is_skipped
    // ------------------------------------------------------------------
    #[test]
    fn already_excluded_entry_is_skipped() {
        let w = worker(1);
        let mut history = vec![ChatEntry::user("anchor")];
        let mut asst = large_assistant();
        asst.context_override = ContextOverride::ForcedExclude;
        let asst_id = asst.id.clone();
        history.push(asst);
        // Push the assistant outside radius.
        history.extend(std::iter::repeat_n(trivial_assistant("x"), 5));

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(
            !excluded.contains(&asst_id),
            "already-excluded entry must not receive duplicate mutation"
        );
    }

    // ------------------------------------------------------------------
    // 12. pinned_entry_is_skipped
    // ------------------------------------------------------------------
    #[test]
    fn pinned_entry_is_skipped() {
        use crate::feat::session::chat_entry::PinPosition;
        let w = worker(1);
        let mut history = vec![ChatEntry::user("anchor")];
        let mut asst = large_assistant();
        asst.pin_position = Some(PinPosition::Top);
        let asst_id = asst.id.clone();
        history.push(asst);
        history.extend(std::iter::repeat_n(trivial_assistant("x"), 5));

        let mutations = evaluate(&w, history);
        assert!(
            !excluded_ids(&mutations).contains(&asst_id),
            "pinned entry must not be pruned"
        );
    }

    // ------------------------------------------------------------------
    // 13. empty_assistant_is_skipped
    // ------------------------------------------------------------------
    #[test]
    fn empty_assistant_is_skipped() {
        let w = worker(1);
        let mut history = vec![ChatEntry::user("anchor")];
        history.push(trivial_assistant(""));
        history.extend(std::iter::repeat_n(trivial_assistant("x"), 5));

        // No mutations at all — the empty assistant is skipped, the trivial
        // ones are filtered by token count.
        let mutations = evaluate(&w, history);
        assert!(mutations.is_empty());
    }

    // ------------------------------------------------------------------
    // 11b. small_assistant_far_from_user_is_not_pruned
    //
    // Small (<=80 tokens) assistant far from any user must NOT be
    // pruned by this worker — that's the trivial_assistant worker's
    // job. Verifies the disjoint-candidate-set contract.
    // ------------------------------------------------------------------
    #[test]
    fn small_assistant_far_from_user_is_not_pruned() {
        let w = worker(2);
        let mut history = vec![ChatEntry::user("anchor at 0")];
        // 100 entries of padding so the small assistant is far from any user.
        history.extend(std::iter::repeat_n(large_assistant(), 100));
        let small = trivial_assistant("ok");
        let small_id = small.id.clone();
        history.push(small);

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(
            !excluded.contains(&small_id),
            "small (<=80 token) assistant must not be pruned by this worker"
        );
    }

    // ------------------------------------------------------------------
    // 14. non_assistant_entries_in_prune_region_not_targeted
    //
    // ToolCall, ToolResult, System, etc. must not receive mutations.
    // ------------------------------------------------------------------
    #[test]
    fn non_assistant_entries_in_prune_region_not_targeted() {
        use crate::feat::session::tool_result_status::ToolResultStatus;
        let w = worker(1);
        let mut history = vec![ChatEntry::user("start")];
        history.push(ChatEntry::system("sys"));
        history.push(ChatEntry::error("err"));
        history.push(ChatEntry::tool_call("c1", "bash", r#"{"command":"ls"}"#));
        history.push(ChatEntry::tool_result("c1", "bash", "out", ToolResultStatus::Success));
        history.push(large_assistant()); // <- candidate, far from user
        history.push(large_assistant()); // <- candidate
        // Push them beyond radius with trivial padding.
        history.extend(std::iter::repeat_n(trivial_assistant("x"), 5));

        let mutations = evaluate(&w, history.clone());
        // The only excluded ids must belong to Assistant entries.
        for m in &mutations {
            if let HistoryMutation::SetContextOverride { entry_id, .. } = m {
                let found = history
                    .iter()
                    .find(|e| e.id == *entry_id)
                    .expect("mutation targets entry in history");
                assert!(
                    matches!(found.kind, ChatEntryKind::Assistant(_)),
                    "non-assistant entry must not be targeted: {:?}",
                    found.kind
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // 15. token_cache_populated_after_evaluate
    //
    // First evaluate populates the cache; second evaluate uses cache.
    // Verify by intercepting with a known token-count and observing the
    // cached value.
    // ------------------------------------------------------------------
    #[test]
    fn token_cache_populated_after_evaluate() {
        let w = worker(1);
        let session_id = SessionId::new();
        let asst = large_assistant();
        let asst_id = asst.id.clone();
        let history: Arc<[ChatEntry]> =
            vec![ChatEntry::user("anchor"), asst].into();

        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async { w.evaluate(&session_id, history.clone()).await });

        let cached = w.token_cache.get(&session_id, &asst_id);
        assert!(
            cached.is_some(),
            "assistant entry must be cached after evaluate"
        );
        let cached_count = cached.expect("checked Some");
        assert!(
            cached_count >= MIN_CANDIDATE_TOKENS,
            "large_assistant helper must produce >80 tokens, got {cached_count}"
        );
    }

    // ------------------------------------------------------------------
    // 15b. second_evaluate_uses_cached_tokens_not_recomputed
    //
    // Accepcance criterion AC6 requires that the second evaluation of a
    // session reads tokens from the cache rather than recomputing them.
    //
    // Proof strategy: first call populates the cache with the true count
    // (>80). We then *sabotage* the cache by overwriting the count with a
    // small value (10). If the second call recomputes, it will get >80
    // again and emit a prune mutation. If it reads from cache, it will
    // see 10 tokens and skip the entry entirely.
    //
    // We reset `context_override` to `Default` between calls to avoid the
    // idempotency skip hiding the recomputation.
    // ------------------------------------------------------------------
    #[test]
    fn second_evaluate_uses_cached_tokens_not_recomputed() {
        let w = worker(1);
        let session_id = SessionId::new();
        let asst = large_assistant();
        let asst_id = asst.id.clone();
        let history: Arc<[ChatEntry]> =
            vec![ChatEntry::user("anchor"), asst].into();

        let rt = tokio::runtime::Runtime::new().expect("runtime");

        // First call populates the cache and (would) emit a prune mutation.
        let _ = rt.block_on(async { w.evaluate(&session_id, history.clone()).await });

        // Sanity: cache now holds a real count >80.
        let cached = w
            .token_cache
            .get(&session_id, &asst_id)
            .expect("first evaluate must populate cache");
        assert!(
            cached >= MIN_CANDIDATE_TOKENS,
            "real token count must be >80, got {cached}"
        );

        // Sabotage the cache so the cached value is now below threshold.
        w.token_cache
            .insert(session_id.clone(), asst_id.clone(), MIN_CANDIDATE_TOKENS - 1);

        // Reset context_override on a fresh history copy so the worker
        // doesn't take the idempotency path.
        let mut history2 = (*history).to_vec();
        for e in &mut history2 {
            e.context_override = ContextOverride::Default;
        }

        let mutations =
            rt.block_on(async { w.evaluate(&session_id, history2.into()).await });
        let excluded = excluded_ids(&mutations);
        assert!(
            !excluded.contains(&asst_id),
            "second evaluate must read sabotaged cache and skip the entry; \
             if it recomputed, the entry would be pruned again"
        );
    }

    // ------------------------------------------------------------------
    // 16. radius_clamped_to_minimum_one
    // ------------------------------------------------------------------
    #[test]
    fn radius_clamped_to_minimum_one() {
        let w = worker(0);
        let mut history = vec![ChatEntry::user("anchor")];
        // Two large assistants: at d=1 (kept even at R=1) and d=2 (pruned).
        let near = large_assistant();
        let near_id = near.id.clone();
        history.push(near);
        let far = large_assistant();
        let far_id = far.id.clone();
        history.push(far);

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(!excluded.contains(&near_id), "d=1 at clamped R=1 must be kept");
        assert!(excluded.contains(&far_id), "d=2 at clamped R=1 must be pruned");
    }

    // ------------------------------------------------------------------
    // 17. multiple_large_assistants_far_from_user_all_pruned
    // ------------------------------------------------------------------
    #[test]
    fn multiple_large_assistants_far_from_user_all_pruned() {
        let w = worker(5);
        let mut history = vec![ChatEntry::user("start")];
        // First response: kept (d=1).
        let kept = large_assistant();
        let kept_id = kept.id.clone();
        history.push(kept);
        // 50 trivial entries → push the next large assistants far from user.
        history.extend(std::iter::repeat_n(trivial_assistant("x"), 50));
        // 5 large assistants — all far.
        let mut pruned_ids = Vec::new();
        for _ in 0..5 {
            let a = large_assistant();
            pruned_ids.push(a.id.clone());
            history.push(a);
        }

        let mutations = evaluate(&w, history);
        let excluded = excluded_ids(&mutations);
        assert!(!excluded.contains(&kept_id));
        for id in &pruned_ids {
            assert!(excluded.contains(id), "expected {id} to be pruned");
        }
    }

    // ------------------------------------------------------------------
    // 18. idempotency_second_evaluate_no_new_mutations
    //
    // After applying mutations to history, second evaluate must produce
    // zero new mutations.
    // ------------------------------------------------------------------
    #[test]
    fn idempotency_second_evaluate_no_new_mutations() {
        let w = worker(2);
        let mut history = vec![ChatEntry::user("anchor")];
        let asst1 = large_assistant();
        let asst1_id = asst1.id.clone();
        history.push(asst1);
        history.push(trivial_assistant("filler"));
        let asst2 = large_assistant();
        let asst2_id = asst2.id.clone();
        history.push(asst2);
        // Push asst2 outside radius.
        history.push(trivial_assistant("more filler"));

        let first = evaluate(&w, history.clone());
        assert!(excluded_ids(&first).contains(&asst2_id));
        assert!(!excluded_ids(&first).contains(&asst1_id));

        // Apply mutations to a copy of history.
        let mut applied = history.clone();
        for m in &first {
            if let HistoryMutation::SetContextOverride { entry_id, value } = m
                && let Some(e) = applied.iter_mut().find(|e| e.id == *entry_id)
            {
                e.context_override = *value;
            }
        }

        let second = evaluate(&w, applied);
        assert!(
            second.is_empty(),
            "second evaluate after applying mutations must produce no new mutations"
        );
    }

    // ------------------------------------------------------------------
    // 19. distances_to_nearest_users_unit_tests
    //
    // Direct unit tests for the helper.
    // ------------------------------------------------------------------
    #[test]
    fn distances_helper_empty_users_returns_none_none() {
        let (a, b) = distances_to_nearest_users(5, &[]);
        assert_eq!((a, b), (None, None));
    }

    #[test]
    fn distances_helper_user_before_returns_back_only() {
        let users = vec![0_usize, 10];
        let (a, b) = distances_to_nearest_users(3, &users);
        assert_eq!((a, b), (Some(3), Some(7)));
    }

    #[test]
    fn distances_helper_user_after_returns_fwd_only() {
        let users = vec![5_usize];
        let (a, b) = distances_to_nearest_users(2, &users);
        assert_eq!((a, b), (None, Some(3)));
    }

    #[test]
    fn distances_helper_users_both_sides_returns_both() {
        let users = vec![2_usize, 10, 20];
        let (a, b) = distances_to_nearest_users(12, &users);
        assert_eq!((a, b), (Some(2), Some(8)));
    }
}
