//! Compaction algorithm — boundary finding, token accumulation, entry gathering.
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

    // If all tool calls have matching results, the loop is complete — safe to cut here.
    if tool_call_ids.is_empty()
        || tool_call_ids
            .iter()
            .all(|id| tool_result_ids.iter().any(|r| r == id))
    {
        return cut_index;
    }

    // Incomplete tool loop — walk forward past the entire group and re-check.
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
    use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride};

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
        let entries = vec![
            ChatEntry::system("ready"),
            ChatEntry::user("hello"),
        ];
        let (indices, _) = gather_compactable_entries(&entries, 0, 2);
        assert_eq!(indices.len(), 1);
        assert_eq!(indices[0], 1); // Only the user entry
    }
}
