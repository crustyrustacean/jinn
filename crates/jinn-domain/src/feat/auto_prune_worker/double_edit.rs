//! Double-edit auto-prune worker.
//!
//! Caps the number of `edit` and `write` tool call+result pairs per file path.
//! When the count for a file exceeds `max_file_edits`, the oldest pairs are
//! **immediately** excluded from context (both `ToolCall` and `ToolResult`).
//! No tail-entry delay.
//!
//! # Example
//!
//! ```text
//! X  [Tool Call]: edit(/foo.rs)     ← pruned (oldest)
//! X  [Tool Result] (OK): applied   ← pruned
//!    [Tool Call]: write(/foo.rs)    ← kept
//!    [Tool Result] (OK): written   ← kept
//!    [Tool Call]: edit(/foo.rs)     ← kept (newest)
//!    [Tool Result] (OK): applied   ← kept
//! ```
//!
//! With `max_file_edits = 2`, the oldest edit/write pair is pruned.
//!
//! [`ForcedExclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedExclude

use std::collections::HashMap;
use std::sync::Arc;

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::DoubleEditAutoPruneConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

/// Double-edit auto-prune worker.
///
/// Inspects history for `edit` and `write` tool calls grouped by file path.
/// When more than `max_file_edits` pairs exist for a single path, the oldest
/// pairs are excluded from context immediately (no tail-entry threshold).
#[derive(Clone)]
pub struct DoubleEditAutoPruneWorker {
    /// Configuration for the double-edit auto-prune strategy.
    pub config: DoubleEditAutoPruneConfig,
}

/// Extract the `path` field from a tool call's JSON arguments string.
///
/// Returns `None` if the arguments cannot be parsed or the `path` field is
/// missing or not a string.
fn extract_path_from_arguments(arguments: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    value
        .get("path")?
        .as_str()
        .map(std::borrow::ToOwned::to_owned)
}

/// Walk forward from a ToolCall to find its matching ToolResult.
///
/// Returns `None` if the result is missing, pending, or already excluded.
fn find_matching_result(
    history: &[ChatEntry],
    call_idx: usize,
    tool_call_id: &str,
) -> Option<ChatEntryId> {
    // ToolResults appear after their ToolCall, so scan forward only.
    for entry in history.iter().skip(call_idx + 1) {
        if let ChatEntryKind::ToolResult { id, .. } = &entry.kind
            && id == tool_call_id
        {
            // If the result was already excluded by a prior prune, the whole
            // pair is already handled — stop looking (don't fall through to a
            // later ToolResult with the same ID, which shouldn't exist but we
            // guard against it by returning here).
            if entry.context_override == ContextOverride::ForcedExclude {
                return None;
            }
            return Some(entry.id.clone());
        }
    }
    // No matching result found — the call is still pending or orphaned.
    None
}


/// A collected edit/write tool call paired with its result.
struct EditWritePair {
    call_entry_id: ChatEntryId,
    result_entry_id: ChatEntryId,
}

#[async_trait::async_trait]
impl HistoryWorker for DoubleEditAutoPruneWorker {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "auto-prune-double-edit"
    }

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        if self.config.max_file_edits == 0 {
            return Vec::new();
        }

        let groups = collect_edit_write_pairs_by_path(&history);
        build_prune_mutations(groups, self.config.max_file_edits)
    }
}

/// Scan history for edit/write ToolCalls, resolve each to its result,
/// and group into pairs by file path.
///
/// Skips ToolCalls that are already excluded or have no matching ToolResult.
fn collect_edit_write_pairs_by_path(history: &[ChatEntry]) -> HashMap<String, Vec<EditWritePair>> {
    // First pass: collect every edit/write ToolCall with its history index
    // and file path. We record indices so the second pass can scan forward
    // from each call to find its matching ToolResult.
    let mut candidates: Vec<(usize, String)> = Vec::new();
    for (i, entry) in history.iter().enumerate() {
        if let ChatEntryKind::ToolCall {
            name, arguments, ..
        } = &entry.kind
            && (name == "edit" || name == "write")
            && let Some(path) = extract_path_from_arguments(arguments)
        {
            candidates.push((i, path));
        }
    }

    // Second pass: for each candidate, resolve it to a complete call+result
    // pair and bucket by file path.
    let mut groups: HashMap<String, Vec<EditWritePair>> = HashMap::new();

    for (call_idx, path) in &candidates {
        let call_entry = &history[*call_idx];

        // Already excluded by a prior prune — don't count toward the total.
        if call_entry.context_override == ContextOverride::ForcedExclude {
            continue;
        }

        let ChatEntryKind::ToolCall {
            id: tool_call_id, ..
        } = &call_entry.kind
        else {
            continue;
        };

        // Walk forward to find the ToolResult that matches this call.
        // If none found (pending, orphaned, or already excluded), skip it —
        // incomplete pairs don't count toward the file's total.
        let Some(result_id) = find_matching_result(history, *call_idx, tool_call_id) else {
            continue;
        };

        groups.entry(path.clone()).or_default().push(EditWritePair {
            call_entry_id: call_entry.id.clone(),
            result_entry_id: result_id,
        });
    }

    groups
}

/// For each path group exceeding the limit, emit `SetContextOverride` mutations
/// for the oldest pairs.
fn build_prune_mutations(
    groups: HashMap<String, Vec<EditWritePair>>,
    max_file_edits: usize,
) -> Vec<HistoryMutation> {
    let mut mutations = Vec::new();

    for (_path, pairs) in groups {
        if pairs.len() <= max_file_edits {
            continue;
        }

        // Pairs are in history order (oldest first), so .take() selects the
        // oldest pairs to prune. Both the call and its result must be excluded
        // so the pruned edit/write disappears entirely from context.
        let to_prune = pairs.len() - max_file_edits;
        for pair in pairs.iter().take(to_prune) {
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: pair.call_entry_id.clone(),
                value: ContextOverride::ForcedExclude,
            });
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: pair.result_entry_id.clone(),
                value: ContextOverride::ForcedExclude,
            });
        }
    }

    mutations
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::protocol::SessionId;

    /// Helper: create an edit ToolCall + ToolResult pair.
    fn edit_call_result(call_id: &str, path: &str, content: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "edit", format!(r#"{{"path": "{path}"}}"#)),
            ChatEntry::tool_result(call_id, "edit", content, ToolResultStatus::Success),
        ]
    }

    /// Helper: create a write ToolCall + ToolResult pair.
    fn write_call_result(call_id: &str, path: &str, content: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "write", format!(r#"{{"path": "{path}"}}"#)),
            ChatEntry::tool_result(call_id, "write", content, ToolResultStatus::Success),
        ]
    }

    fn worker_with_max(max: usize) -> DoubleEditAutoPruneWorker {
        DoubleEditAutoPruneWorker {
            config: DoubleEditAutoPruneConfig {
                enabled: true,
                max_file_edits: max,
            },
        }
    }

    fn block_on_evaluate(
        worker: &DoubleEditAutoPruneWorker,
        history: Vec<ChatEntry>,
    ) -> Vec<HistoryMutation> {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async { worker.evaluate(&SessionId::new(), Arc::from(history)).await })
    }

    fn collect_pruned_ids(mutations: Vec<HistoryMutation>) -> Vec<ChatEntryId> {
        let mut ids: Vec<_> = mutations
            .into_iter()
            .map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, value } => {
                    assert_eq!(value, ContextOverride::ForcedExclude);
                    entry_id
                }
                other => panic!("expected SetContextOverride, got {other:?}"),
            })
            .collect();
        ids.sort_by_key(std::string::ToString::to_string);
        ids
    }

    #[test]
    fn no_edit_write_produces_no_mutations() {
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::assistant("hi"),
            ChatEntry::user("what is 2+2?"),
            ChatEntry::assistant("4"),
        ];
        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn under_max_produces_no_mutations() {
        let mut history = Vec::new();
        let e1 = edit_call_result("tc-1", "/foo.rs", "edit 1");
        history.push(e1[0].clone());
        history.push(e1[1].clone());
        let e2 = edit_call_result("tc-2", "/foo.rs", "edit 2");
        history.push(e2[0].clone());
        history.push(e2[1].clone());

        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn over_max_prunes_oldest() {
        let mut history = Vec::new();
        let e1 = edit_call_result("tc-1", "/foo.rs", "edit 1");
        history.push(e1[0].clone());
        history.push(e1[1].clone());
        let e2 = edit_call_result("tc-2", "/foo.rs", "edit 2");
        history.push(e2[0].clone());
        history.push(e2[1].clone());
        let e3 = edit_call_result("tc-3", "/foo.rs", "edit 3");
        history.push(e3[0].clone());
        history.push(e3[1].clone());

        let expected_call_id = history[0].id.clone();
        let expected_result_id = history[1].id.clone();

        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        let pruned_ids = collect_pruned_ids(mutations);

        let mut expected = vec![expected_call_id, expected_result_id];
        expected.sort_by_key(std::string::ToString::to_string);
        assert_eq!(pruned_ids, expected);
    }

    #[test]
    fn mixed_edit_and_write_count_together() {
        let mut history = Vec::new();
        // edit (oldest) + write + write = 3, max=2 → prune the edit
        let e1 = edit_call_result("tc-1", "/foo.rs", "edit 1");
        history.push(e1[0].clone());
        history.push(e1[1].clone());
        let w1 = write_call_result("tc-2", "/foo.rs", "write 1");
        history.push(w1[0].clone());
        history.push(w1[1].clone());
        let w2 = write_call_result("tc-3", "/foo.rs", "write 2");
        history.push(w2[0].clone());
        history.push(w2[1].clone());

        let expected_call_id = history[0].id.clone();
        let expected_result_id = history[1].id.clone();

        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        let pruned_ids = collect_pruned_ids(mutations);

        let mut expected = vec![expected_call_id, expected_result_id];
        expected.sort_by_key(std::string::ToString::to_string);
        assert_eq!(pruned_ids, expected);
    }

    #[test]
    fn different_files_independent() {
        let mut history = Vec::new();
        // 3 edits to /foo.rs, 1 edit to /bar.rs, max=2
        let e1 = edit_call_result("tc-1", "/foo.rs", "edit 1");
        history.push(e1[0].clone());
        history.push(e1[1].clone());
        let b1 = edit_call_result("tc-2", "/bar.rs", "bar edit 1");
        history.push(b1[0].clone());
        history.push(b1[1].clone());
        let e2 = edit_call_result("tc-3", "/foo.rs", "edit 2");
        history.push(e2[0].clone());
        history.push(e2[1].clone());
        let e3 = edit_call_result("tc-4", "/foo.rs", "edit 3");
        history.push(e3[0].clone());
        history.push(e3[1].clone());

        // Only the oldest /foo.rs pair should be pruned.
        let expected_call_id = history[0].id.clone();
        let expected_result_id = history[1].id.clone();

        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        let pruned_ids = collect_pruned_ids(mutations);

        let mut expected = vec![expected_call_id, expected_result_id];
        expected.sort_by_key(std::string::ToString::to_string);
        assert_eq!(pruned_ids, expected);
    }

    #[test]
    fn already_excluded_skipped() {
        let mut history = Vec::new();
        let e1 = edit_call_result("tc-1", "/foo.rs", "edit 1");
        let mut e1_call = e1[0].clone();
        e1_call.context_override = ContextOverride::ForcedExclude;
        history.push(e1_call);
        history.push(e1[1].clone());
        let e2 = edit_call_result("tc-2", "/foo.rs", "edit 2");
        history.push(e2[0].clone());
        history.push(e2[1].clone());
        let e3 = edit_call_result("tc-3", "/foo.rs", "edit 3");
        history.push(e3[0].clone());
        history.push(e3[1].clone());

        // 1 already excluded + 2 in-context = exactly max, nothing to prune.
        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn tool_call_without_result_ignored() {
        let mut history = Vec::new();
        // Orphan edit ToolCall (no result).
        history.push(ChatEntry::tool_call(
            "tc-orphan",
            "edit",
            r#"{"path": "/foo.rs"}"#,
        ));
        // Two complete edits.
        let e1 = edit_call_result("tc-1", "/foo.rs", "edit 1");
        history.push(e1[0].clone());
        history.push(e1[1].clone());
        let e2 = edit_call_result("tc-2", "/foo.rs", "edit 2");
        history.push(e2[0].clone());
        history.push(e2[1].clone());

        // Only 2 complete pairs, max=2 → nothing pruned.
        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn exact_max_no_prune() {
        let mut history = Vec::new();
        let e1 = edit_call_result("tc-1", "/foo.rs", "edit 1");
        history.push(e1[0].clone());
        history.push(e1[1].clone());
        let e2 = edit_call_result("tc-2", "/foo.rs", "edit 2");
        history.push(e2[0].clone());
        history.push(e2[1].clone());

        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn max_zero_means_no_limit() {
        let mut history = Vec::new();
        for i in 0..5 {
            let e = edit_call_result(&format!("tc-{i}"), "/foo.rs", &format!("edit {i}"));
            history.push(e[0].clone());
            history.push(e[1].clone());
        }

        let worker = worker_with_max(0);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn five_edits_prune_three_oldest() {
        let mut history = Vec::new();
        let mut oldest_ids = Vec::new();
        for i in 0..5 {
            let e = edit_call_result(&format!("tc-{i}"), "/foo.rs", &format!("edit {i}"));
            if i < 3 {
                oldest_ids.push(e[0].id.clone());
                oldest_ids.push(e[1].id.clone());
            }
            history.push(e[0].clone());
            history.push(e[1].clone());
        }

        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        assert_eq!(mutations.len(), 6, "3 oldest pairs × 2 = 6 mutations");

        let pruned_ids = collect_pruned_ids(mutations);
        oldest_ids.sort_by_key(std::string::ToString::to_string);
        assert_eq!(pruned_ids, oldest_ids);
    }

    #[test]
    fn multiple_files_both_overflow() {
        let mut history = Vec::new();
        // 4 edits to /a.rs
        for i in 0..4 {
            let e = edit_call_result(&format!("tc-a{i}"), "/a.rs", &format!("a{i}"));
            history.push(e[0].clone());
            history.push(e[1].clone());
        }
        // 3 edits to /b.rs
        for i in 0..3 {
            let e = edit_call_result(&format!("tc-b{i}"), "/b.rs", &format!("b{i}"));
            history.push(e[0].clone());
            history.push(e[1].clone());
        }

        // /a.rs: 4 entries, keep 2 → prune 2 oldest (4 mutations)
        // /b.rs: 3 entries, keep 2 → prune 1 oldest (2 mutations)
        // Total: 6 mutations
        let worker = worker_with_max(2);
        let mutations = block_on_evaluate(&worker, history);
        assert_eq!(mutations.len(), 6);
    }
}
