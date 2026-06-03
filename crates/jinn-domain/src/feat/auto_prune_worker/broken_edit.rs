//! Broken-edit auto-prune worker.
//!
//! Detects `edit` tool calls whose `ToolResult` has `status: Failure` and marks
//! both the `ToolCall` and `ToolResult` as [`ForcedExclude`] once enough
//! in-context entries have accumulated after the failed edit. This removes
//! useless failed-edit noise from the LLM context window.
//!
//! Pruning does not occur until `min_tail_entries` (default: 10) in-context
//! entries have accumulated after the failed edit.
//!
//! # Example
//!
//! ```text
//!    [User]: fix the bug in /foo.rs
//! X  [Tool Call]: edit(/foo.rs)
//! X  [Tool Result] (Failure): stale anchor
//!    [Tool Call]: edit(/foo.rs)
//!    [Tool Result] (OK): edit applied
//!    [Assistant]: I've fixed the bug.
//!
//! [`ForcedExclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedExclude

use std::sync::Arc;

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::BrokenEditAutoPruneConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::SessionId;

/// Broken-edit auto-prune worker.
///
/// Inspects history for `edit` tool calls whose results failed. Once
/// `min_tail_entries` in-context entries have accumulated after the edit
/// `ToolCall`, both the call and its result are excluded from context.
#[derive(Clone)]
pub struct BrokenEditAutoPruneWorker {
    /// Configuration for the broken-edit auto-prune strategy.
    pub config: BrokenEditAutoPruneConfig,
}

/// Walk forward from an edit ToolCall to find its matching ToolResult.
///
/// Returns `Some((entry_id, status))` if a matching result is found.
/// Returns `None` if no result exists (pending/orphaned) or the result
/// is already excluded (the pair is already handled).
fn find_failed_edit_result(
    history: &[ChatEntry],
    call_idx: usize,
    tool_call_id: &str,
) -> Option<(
    crate::feat::session::chat_entry::ChatEntryId,
    ToolResultStatus,
)> {
    // ToolResults appear after their ToolCall, so scan forward only.
    for entry in history.iter().skip(call_idx + 1) {
        if let ChatEntryKind::ToolResult { id, status, .. } = &entry.kind
            && id == tool_call_id
        {
            // Found the matching result — return its ID and status.
            return Some((entry.id.clone(), *status));
        }
    }
    // No matching result found — the call is still pending or orphaned.
    None
}

/// Count in-context entries that appear after the given index.
///
/// Only entries where [`ChatEntry::is_in_context()`] returns true are counted.
/// Used to determine whether enough conversation has happened after a failed
/// edit to safely prune it.
fn count_in_context_after(history: &[ChatEntry], after_idx: usize) -> usize {
    history[(after_idx + 1)..]
        .iter()
        .filter(|e| e.is_in_context())
        .count()
}

#[async_trait::async_trait]
impl HistoryWorker for BrokenEditAutoPruneWorker {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "auto-prune-broken-edit"
    }

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        let mut mutations = Vec::new();

        for i in 0..history.len() {
            let entry = &history[i];

            // Only interested in "edit" tool calls.
            let tool_call_id = match &entry.kind {
                ChatEntryKind::ToolCall { name, id, .. } if name == "edit" => id.clone(),
                _ => continue,
            };

            // Skip if the call is already excluded by a prior prune.
            if entry.context_override == ContextOverride::ForcedExclude {
                continue;
            }

            let edit_call_entry_id = entry.id.clone();

            // Walk forward to find the ToolResult for this edit call.
            // If none found (pending/orphaned), skip — incomplete pairs
            // can't be pruned.
            let Some((result_id, status)) = find_failed_edit_result(&history, i, &tool_call_id)
            else {
                continue;
            };

            // Only prune failed edits — successful ones are useful context.
            if status != ToolResultStatus::Failure {
                continue;
            }

            // Skip if the result is already excluded — the pair is already handled.
            let result_already_excluded = history
                .iter()
                .skip(i + 1)
                .find(|e| e.id == result_id)
                .is_some_and(|e| e.context_override == ContextOverride::ForcedExclude);
            if result_already_excluded {
                continue;
            }

            // Only prune once enough in-context entries have accumulated after
            // the edit. This ensures the failed edit isn't pruned too early,
            // while the LLM might still benefit from seeing the failure context.
            let in_context_count = count_in_context_after(&history, i);

            if in_context_count >= self.config.min_tail_entries {
                mutations.push(HistoryMutation::SetContextOverride {
                    entry_id: edit_call_entry_id,
                    value: ContextOverride::ForcedExclude,
                });
                mutations.push(HistoryMutation::SetContextOverride {
                    entry_id: result_id,
                    value: ContextOverride::ForcedExclude,
                });
            }
        }

        mutations
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::preferences_actor::user_preferences::BrokenEditAutoPruneConfig;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::protocol::SessionId;

    /// Helper: create a failed edit ToolCall + ToolResult pair.
    fn failed_edit_call_result(call_id: &str, path: &str, error_msg: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "edit", format!(r#"{{"path": "{path}"}}"#)),
            ChatEntry::tool_result(call_id, "edit", error_msg, ToolResultStatus::Failure),
        ]
    }

    /// Helper: create a successful edit ToolCall + ToolResult pair.
    fn successful_edit_call_result(call_id: &str, path: &str, msg: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "edit", format!(r#"{{"path": "{path}"}}"#)),
            ChatEntry::tool_result(call_id, "edit", msg, ToolResultStatus::Success),
        ]
    }

    /// Build a history with a failed edit pair followed by N user entries (in-context).
    fn history_with_failed_edit_and_tail(path: &str, tail_count: usize) -> Vec<ChatEntry> {
        let mut history = Vec::new();
        let edit = failed_edit_call_result("tc-1", path, "edit failed: stale anchor");
        history.push(edit[0].clone());
        history.push(edit[1].clone());
        for i in 0..tail_count {
            history.push(ChatEntry::user(format!("tail message {i}")));
        }
        history
    }

    fn worker_with_tail(tail: usize) -> BrokenEditAutoPruneWorker {
        BrokenEditAutoPruneWorker {
            config: BrokenEditAutoPruneConfig {
                enabled: true,
                min_tail_entries: tail,
            },
        }
    }

    async fn run_evaluate(
        worker: &BrokenEditAutoPruneWorker,
        history: Vec<ChatEntry>,
    ) -> Vec<HistoryMutation> {
        worker.evaluate(&SessionId::new(), Arc::from(history)).await
    }

    fn block_on_evaluate(
        worker: &BrokenEditAutoPruneWorker,
        history: Vec<ChatEntry>,
    ) -> Vec<HistoryMutation> {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async { run_evaluate(worker, history).await })
    }

    // --- Worker evaluate() tests ---

    #[test]
    fn no_edit_produces_no_mutations() {
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::assistant("hi"),
            ChatEntry::user("what is 2+2?"),
            ChatEntry::assistant("4"),
        ];
        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn successful_edit_produces_no_mutations() {
        let mut history = Vec::new();
        let edit = successful_edit_call_result("tc-1", "/foo.rs", "edit applied");
        history.push(edit[0].clone());
        history.push(edit[1].clone());
        for i in 0..10 {
            history.push(ChatEntry::user(format!("tail message {i}")));
        }
        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn failed_edit_with_enough_tail_prunes_both() {
        let history = history_with_failed_edit_and_tail("/foo.rs", 10);
        let expected_call_id = history[0].id.clone();
        let expected_result_id = history[1].id.clone();
        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);

        assert_eq!(mutations.len(), 2);

        let mut pruned_ids: Vec<_> = mutations
            .iter()
            .map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, value } => {
                    assert_eq!(*value, ContextOverride::ForcedExclude);
                    entry_id.clone()
                }
                other => panic!("expected SetContextOverride, got {other:?}"),
            })
            .collect();
        pruned_ids.sort_by_key(std::string::ToString::to_string);

        let mut expected = vec![expected_call_id, expected_result_id];
        expected.sort_by_key(std::string::ToString::to_string);

        assert_eq!(pruned_ids, expected);
    }

    #[test]
    fn tail_below_threshold_does_not_prune() {
        let history = history_with_failed_edit_and_tail("/foo.rs", 5);
        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn already_excluded_call_produces_no_duplicate_mutation() {
        let mut history = history_with_failed_edit_and_tail("/foo.rs", 10);
        // Mark the edit ToolCall as already excluded.
        history[0].context_override = ContextOverride::ForcedExclude;

        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn already_excluded_result_produces_no_duplicate_mutation() {
        let mut history = history_with_failed_edit_and_tail("/foo.rs", 10);
        // Mark the edit ToolResult as already excluded.
        history[1].context_override = ContextOverride::ForcedExclude;

        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn multiple_failed_edits_all_pruned() {
        let mut history = Vec::new();

        // First failed edit.
        let edit1 = failed_edit_call_result("tc-1", "/a.rs", "failed a");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());

        // Second failed edit.
        let edit2 = failed_edit_call_result("tc-2", "/b.rs", "failed b");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());

        // Tail entries after last edit.
        for i in 0..10 {
            history.push(ChatEntry::user(format!("tail {i}")));
        }

        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert_eq!(
            mutations.len(),
            4,
            "both failed edits should be pruned (2 entries each)"
        );
    }

    #[test]
    fn edit_without_result_produces_no_mutation() {
        let mut history = Vec::new();
        // Edit tool call but no corresponding tool result.
        history.push(ChatEntry::tool_call(
            "tc-orphan",
            "edit",
            r#"{"path": "/foo.rs"}"#,
        ));
        for i in 0..10 {
            history.push(ChatEntry::user(format!("tail {i}")));
        }

        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn exact_threshold_prunes() {
        // Exactly min_tail_entries should prune.
        let history = history_with_failed_edit_and_tail("/foo.rs", 10);
        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert_eq!(mutations.len(), 2);
    }

    #[test]
    fn one_below_threshold_does_not_prune() {
        // min_tail_entries=10. History: edit_call, edit_result, 9 users.
        // In-context after edit call: edit_result (1) + 9 users = 10.
        // Actually that's 10 which IS the threshold. Let's use 8 users for 9 total.
        let history = history_with_failed_edit_and_tail("/foo.rs", 8);
        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn only_counts_in_context_entries() {
        // After edit ToolCall: edit_result (in-context) + 8 user (in-context) + 6 transient (not in-context).
        // In-context count = 9 < 10, so no prune.
        let mut history = Vec::new();
        let edit = failed_edit_call_result("tc-1", "/foo.rs", "failed");
        history.push(edit[0].clone());
        history.push(edit[1].clone());
        for i in 0..8 {
            history.push(ChatEntry::user(format!("msg {i}")));
        }
        for i in 0..6 {
            history.push(ChatEntry::transient(format!("transient {i}")));
        }
        // Total after edit ToolCall = 15, but only 9 in-context (edit result + 8 users).

        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert!(
            mutations.is_empty(),
            "should not prune: only 9 in-context entries"
        );
    }

    #[test]
    fn mix_of_success_and_failure_only_prunes_failures() {
        let mut history = Vec::new();

        // Successful edit.
        let ok_edit = successful_edit_call_result("tc-1", "/a.rs", "ok");
        history.push(ok_edit[0].clone());
        history.push(ok_edit[1].clone());

        // Failed edit.
        let fail_edit = failed_edit_call_result("tc-2", "/b.rs", "failed");
        history.push(fail_edit[0].clone());
        history.push(fail_edit[1].clone());

        // Tail entries.
        for i in 0..10 {
            history.push(ChatEntry::user(format!("tail {i}")));
        }

        let worker = worker_with_tail(10);
        let mutations = block_on_evaluate(&worker, history);
        assert_eq!(
            mutations.len(),
            2,
            "only the failed edit pair should be pruned"
        );
    }
}
