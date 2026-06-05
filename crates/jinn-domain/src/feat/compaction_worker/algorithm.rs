//! Compaction algorithm - boundary finding, token accumulation, entry gathering.
//!
//! Extracted from the old `CompactionActor` for reuse by `CompactionWorker`.

use crate::feat::context::strategy::token_estimator::CharRatioEstimator;
use crate::feat::context::strategy::token_estimator::estimate_entry_tokens;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};

/// Adjust the token-based cut index forward to the next valid boundary.
///
/// The cut must not land on a `ToolCall` or `ToolResult` entry, because
/// these have structural dependencies on preceding messages:
///
/// - `ToolCall` merges into the preceding `Assistant` in `entries_to_messages`.
///   If that `Assistant` is compacted away, the tool call becomes orphaned.
/// - `ToolResult` produces a `tool` role message whose `tool_call_id` must
///   match a preceding `assistant.tool_calls[].id`. If the `Assistant` is
///   compacted but the `ToolResult` is kept, the provider rejects the request.
///
/// Walking forward past `ToolCall`/`ToolResult` to the next independent entry
/// (`Assistant`, `User`, `Error`, `System`, `Compaction`, etc.) ensures the
/// kept entries form a structurally valid LLM message sequence.
///
/// Returns the adjusted cut index (>= `cut_index`, <= `history.len()`).
pub fn adjust_cut_to_boundary(history: &[ChatEntry], cut_index: usize) -> usize {
    if cut_index >= history.len() {
        return cut_index;
    }

    // Pass 1: If cut lands on a ToolCall or ToolResult, walk forward past them.
    let cut_index = if matches!(
        history[cut_index].kind,
        ChatEntryKind::ToolCall { .. } | ChatEntryKind::ToolResult { .. }
    ) {
        history[cut_index..]
            .iter()
            .position(|entry| {
                !matches!(
                    entry.kind,
                    ChatEntryKind::ToolCall { .. } | ChatEntryKind::ToolResult { .. }
                )
            })
            .map_or(history.len(), |offset| cut_index + offset)
    } else {
        cut_index
    };

    if cut_index >= history.len() {
        return cut_index;
    }

    // Pass 2: If the cut lands on an Assistant, check for incomplete tool loops.
    if !matches!(history[cut_index].kind, ChatEntryKind::Assistant(..)) {
        return cut_index;
    }

    // Scan the tool loop group starting from this Assistant.
    let mut tool_call_ids: Vec<String> = Vec::new();
    let mut tool_result_ids: Vec<String> = Vec::new();
    let mut group_end = cut_index + 1;

    for (offset, entry) in history[cut_index + 1..].iter().enumerate() {
        match &entry.kind {
            ChatEntryKind::ToolCall { id, .. } => {
                tool_call_ids.push(id.clone());
                group_end = cut_index + 1 + offset + 1;
            }
            ChatEntryKind::ToolResult { id, .. } => {
                tool_result_ids.push(id.clone());
                group_end = cut_index + 1 + offset + 1;
            }
            // Stop at any entry that isn't part of this tool loop group.
            _ => break,
        }
    }

    // If all tool calls have matching results, the loop is complete - safe to cut here.
    if tool_call_ids.is_empty()
        || tool_call_ids
            .iter()
            .all(|id| tool_result_ids.iter().any(|r| r == id))
    {
        return cut_index;
    }

    // Incomplete tool loop - walk forward past the entire group and re-check.
    adjust_cut_to_boundary(history, group_end)
}

/// Compute the cut index by walking backwards accumulating tokens.
///
/// Returns the index where recent entries begin (those fitting within the reserve).
/// If `compact_all` is true, returns `history.len()`.
/// If all tokens fit within the reserve, returns `start_index`.
pub fn compute_cut_index(
    history: &[ChatEntry],
    start_index: usize,
    reserve_tokens: usize,
    compact_all: bool,
) -> usize {
    let estimator = CharRatioEstimator;

    if compact_all {
        return history.len();
    }

    let mut accumulated_tokens = 0usize;
    let mut cut_index = start_index;

    for i in (start_index..history.len()).rev() {
        let entry = &history[i];
        let tokens = estimate_entry_tokens(&estimator, entry);
        accumulated_tokens += tokens;
        if accumulated_tokens > reserve_tokens {
            cut_index = i + 1;
            break;
        }
    }

    cut_index
}

/// Gather compactable entries between `start_index` and `cut_index`,
/// excluding System and Compaction entries.
///
/// Returns `(gathered_indices, tokens_before)` where `tokens_before` is
/// the total estimated tokens of the gathered entries.
pub fn gather_compactable_entries(
    history: &[ChatEntry],
    start_index: usize,
    cut_index: usize,
) -> (Vec<usize>, usize) {
    let estimator = CharRatioEstimator;
    let mut gathered_indices: Vec<usize> = Vec::new();
    let mut tokens_before: usize = 0;
    for (i, entry) in history.iter().enumerate().take(cut_index).skip(start_index) {
        if matches!(entry.kind, ChatEntryKind::System(_)) || entry.is_compaction() {
            continue;
        }
        tokens_before += estimate_entry_tokens(&estimator, entry);
        gathered_indices.push(i);
    }
    (gathered_indices, tokens_before)
}

/// Resolve the effective context window size.
///
/// Uses the provider-reported `context_length` if available,
/// otherwise falls back to the configured `fallback`.
pub fn resolve_context_window(context_length: Option<u32>, fallback: usize) -> usize {
    context_length.map_or(fallback, |v| v as usize)
}

/// Estimate total tokens for all entries from `start_index` to end.
///
/// Excludes `Compaction` entries (they are boundaries, not content).
/// Includes System entries - they consume context window budget even though
/// they're excluded from compaction by `gather_compactable_entries()`.
pub fn estimate_total_tokens(history: &[ChatEntry], start_index: usize) -> usize {
    let estimator = CharRatioEstimator;
    let mut total = 0usize;
    for entry in history.iter().take(history.len()).skip(start_index) {
        if entry.is_compaction() {
            continue;
        }
        total += estimate_entry_tokens(&estimator, entry);
    }
    total
}

/// Find the start boundary: index after the last Compaction entry.
pub fn find_start_boundary(history: &[ChatEntry]) -> usize {
    history
        .iter()
        .rposition(ChatEntry::is_compaction)
        .map_or(0, |i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feat::session::chat_entry::{
        ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride,
    };

    #[test]
    fn find_start_boundary_returns_zero_when_no_compaction() {
        let entries = vec![ChatEntry::user("hello"), ChatEntry::assistant("hi")];
        assert_eq!(find_start_boundary(&entries), 0);
    }

    #[test]
    fn find_start_boundary_returns_after_last_compaction() {
        let entries = vec![
            ChatEntry::user("hello"),
            ChatEntry {
                id: ChatEntryId::new(),
                timestamp: jiff::Timestamp::now(),
                kind: ChatEntryKind::Compaction {
                    summary: "summary1".to_owned(),
                    tokens_before: 0,
                    tokens_after: 0,
                    entries_compacted: 0,
                    model_used: "model".to_owned(),
                },
                pin_position: None,
                context_override: ContextOverride::Default,
            context_history: Vec::new(),
            },
            ChatEntry::user("world"),
        ];
        assert_eq!(find_start_boundary(&entries), 2);
    }

    #[test]
    fn compute_cut_index_returns_len_for_compact_all() {
        let entries = vec![ChatEntry::user("hello"), ChatEntry::assistant("hi")];
        assert_eq!(compute_cut_index(&entries, 0, 100, true), 2);
    }

    #[test]
    fn compute_cut_index_returns_start_when_all_fit() {
        let entries = vec![ChatEntry::user("hi")];
        // A short entry fits in 10000 tokens reserve.
        assert_eq!(compute_cut_index(&entries, 0, 10000, false), 0);
    }

    #[test]
    fn gather_compactable_excludes_system() {
        let entries = vec![ChatEntry::system("ready"), ChatEntry::user("hello")];
        let (indices, _) = gather_compactable_entries(&entries, 0, 2);
        assert_eq!(indices.len(), 1);
        assert_eq!(indices[0], 1); // Only the user entry
    }

    #[test]
    fn adjust_cut_walks_past_tool_call_group() {
        // Given history: [User, Assistant, ToolCall, ToolResult, Assistant, ToolCall]
        // Cut at index 2 lands on ToolCall - should walk forward past entire tool loop group.
        // The Assistant at index 4 starts an incomplete tool loop (ToolCall at 5 has no result),
        // so the adjustment walks past it to index 6 (history.len()).
        let entries = vec![
            ChatEntry::user("do something"),
            ChatEntry::assistant("let me check"),
            ChatEntry::tool_call("tc1", "bash", r#"{"command":"ls"}"#),
            ChatEntry::tool_result(
                "tc1",
                "bash",
                "file.txt",
                crate::feat::session::tool_result_status::ToolResultStatus::Success,
            ),
            ChatEntry::assistant("here is the result"),
            ChatEntry::tool_call("tc2", "read", r#"{"path":"file.txt"}"#),
        ];

        // When adjusting cut at index 2 (lands on ToolCall).
        let adjusted = adjust_cut_to_boundary(&entries, 2);

        // Then it walks forward past both tool groups. The first group (tc1)
        // is complete, but the second group (tc2, no result) is incomplete.
        // The result is past the end of history (6 = len).
        assert_eq!(
            adjusted, 6,
            "should skip past both tool groups to end of history"
        );
    }

    #[test]
    fn adjust_cut_stops_at_complete_tool_loop() {
        // Given history: [User, Assistant, ToolCall, ToolResult, Assistant("done")]
        // Cut at index 2 lands on ToolCall - the tool loop IS complete (result present).
        // Pass 1 walks to index 4 (Assistant). Pass 2 finds no tool calls, so the
        // Assistant at 4 is clean - cut stays at 4.
        let entries = vec![
            ChatEntry::user("do something"),
            ChatEntry::assistant("let me check"),
            ChatEntry::tool_call("tc1", "bash", r#"{"command":"ls"}"#),
            ChatEntry::tool_result(
                "tc1",
                "bash",
                "file.txt",
                crate::feat::session::tool_result_status::ToolResultStatus::Success,
            ),
            ChatEntry::assistant("done"),
        ];

        // When adjusting cut at index 2.
        let adjusted = adjust_cut_to_boundary(&entries, 2);

        // Then it stops at the Assistant at index 4 - the tool loop is complete.
        assert_eq!(
            adjusted, 4,
            "should stop at Assistant after complete tool loop"
        );
    }

    #[test]
    fn adjust_cut_walks_past_incomplete_tool_loop() {
        // Given history: [User, Assistant, ToolCall] (no ToolResult yet)
        // Cut at index 2 lands on ToolCall - incomplete loop should skip it.
        let entries = vec![
            ChatEntry::user("do something"),
            ChatEntry::assistant("let me check"),
            ChatEntry::tool_call("tc1", "bash", r#"{"command":"ls"}"#),
        ];

        // When adjusting cut at index 2.
        let adjusted = adjust_cut_to_boundary(&entries, 2);

        // Then it walks past the incomplete tool loop to the end of history.
        assert_eq!(adjusted, 3, "should skip past incomplete tool loop to end");
    }

    #[test]
    fn compute_cut_index_walks_backwards_from_reserve() {
        // Given entries with enough text that they won't all fit in a tiny reserve.
        // Each entry ~70 chars = ~18 tokens via char-ratio (0.25 ratio).
        let entries = vec![
            ChatEntry::user("msg 0 is about twenty tokens long enough to be more than ten"),
            ChatEntry::assistant("resp 0 about twenty tokens long enough to be more than ten"),
            ChatEntry::user("msg 1 is about twenty tokens long enough to be more than ten"),
            ChatEntry::assistant("resp 1 about twenty tokens long enough to be more than ten"),
            ChatEntry::user("msg 2 is about twenty tokens long enough to be more than ten"),
            ChatEntry::assistant("resp 2 about twenty tokens long enough to be more than ten"),
        ];

        // When computing cut with a tiny reserve that can only hold ~2 entries.
        let cut = compute_cut_index(&entries, 0, 30, false);

        // Then the cut is past the start - older entries don't fit in reserve.
        assert!(
            cut > 0,
            "cut should be past start when entries exceed reserve"
        );
        assert!(
            cut < entries.len(),
            "cut should be before end when some entries fit"
        );
    }
}
