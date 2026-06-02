//! Consecutive-reads auto-prune worker.
//!
//! Detects repeated `read` tool calls on the same file path and keeps only the
//! last `keep_last` (default: 3) call+result pairs per path, marking all older
//! pairs as [`ForcedExclude`]. This removes stale file contents from the LLM
//! context window.
//!
//! Pruning is immediate — no threshold or delay.
//! Path matching is exact string comparison (no normalization).
//!
//! # Example (keep_last = 2)
//!
//! ```text
//! X  [Tool Call]: read("/foo.rs")       ← pruned (3rd oldest)
//! X  [Tool Result] (OK): <old contents>
//! X  [Tool Call]: read("/foo.rs")       ← pruned (2nd oldest)
//! X  [Tool Result] (OK): <old contents>
//!    [Tool Call]: read("/foo.rs")       ← kept (most recent)
//!    [Tool Result] (OK): <current contents>
//!    [Assistant]: based on the latest read...
//! ```
//!
//! [`ForcedExclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedExclude

use std::collections::HashMap;
use std::sync::Arc;

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::ConsecutiveReadsAutoPruneConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

/// Consecutive-reads auto-prune worker.
///
/// For each unique file path, keeps only the last `keep_last` `read` tool
/// call+result pairs. Older pairs are excluded from LLM context.
#[derive(Clone)]
pub struct ConsecutiveReadsAutoPruneWorker {
    /// Configuration for the consecutive-reads auto-prune strategy.
    pub config: ConsecutiveReadsAutoPruneConfig,
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

/// A matched read pair (ToolCall + its corresponding ToolResult).
struct ReadPair {
    call_entry_id: crate::feat::session::chat_entry::ChatEntryId,
    result_entry_id: crate::feat::session::chat_entry::ChatEntryId,
}

#[async_trait::async_trait]
impl HistoryWorker for ConsecutiveReadsAutoPruneWorker {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "auto-prune-consecutive-reads"
    }

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        let keep_last = self.config.keep_last.max(1);

        // Phase 1: Collect all read pairs, grouped by file path.
        let mut pairs_by_path: HashMap<String, Vec<ReadPair>> = HashMap::new();

        for i in 0..history.len() {
            let entry = &history[i];

            // Only interested in "read" tool calls.
            let (tool_call_id, read_path) = match &entry.kind {
                ChatEntryKind::ToolCall {
                    name,
                    arguments,
                    id,
                } if name == "read" => {
                    let Some(path) = extract_path_from_arguments(arguments) else {
                        continue;
                    };
                    (id.clone(), path)
                }
                _ => continue,
            };

            let call_entry_id = entry.id.clone();

            // Find the corresponding ToolResult (matched by tool call id).
            let mut result_entry_id = None;
            for entry_j in history.iter().skip(i + 1) {
                if let ChatEntryKind::ToolResult { id, .. } = &entry_j.kind
                    && id == &tool_call_id
                {
                    result_entry_id = Some(entry_j.id.clone());
                    break;
                }
            }

            let Some(result_id) = result_entry_id else {
                continue;
            };

            pairs_by_path
                .entry(read_path)
                .or_default()
                .push(ReadPair {
                    call_entry_id,
                    result_entry_id: result_id,
                });
        }

        // Phase 2: For each path, prune older pairs exceeding keep_last.
        let mut mutations = Vec::new();

        for (_path, pairs) in &pairs_by_path {
            if pairs.len() <= keep_last {
                continue;
            }

            let prune_count = pairs.len() - keep_last;
            for pair in pairs.iter().take(prune_count) {
                // Only emit mutations for entries not already excluded.
                let call_already_excluded = history.iter().any(|e| {
                    e.id == pair.call_entry_id
                        && e.context_override == ContextOverride::ForcedExclude
                });
                let result_already_excluded = history.iter().any(|e| {
                    e.id == pair.result_entry_id
                        && e.context_override == ContextOverride::ForcedExclude
                });

                if !call_already_excluded {
                    mutations.push(HistoryMutation::SetContextOverride {
                        entry_id: pair.call_entry_id.clone(),
                        value: ContextOverride::ForcedExclude,
                    });
                }
                if !result_already_excluded {
                    mutations.push(HistoryMutation::SetContextOverride {
                        entry_id: pair.result_entry_id.clone(),
                        value: ContextOverride::ForcedExclude,
                    });
                }
            }
        }

        mutations
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::preferences_actor::user_preferences::ConsecutiveReadsAutoPruneConfig;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::protocol::SessionId;

    /// Helper: create a read ToolCall + ToolResult pair.
    fn read_call_result(call_id: &str, path: &str, content: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "read", format!(r#"{{"path": "{path}"}}"#)),
            ChatEntry::tool_result(call_id, "read", content, ToolResultStatus::Success),
        ]
    }

    /// Build a history with N read pairs of the same file.
    fn history_with_n_reads(path: &str, count: usize) -> Vec<ChatEntry> {
        let mut history = Vec::new();
        for i in 0..count {
            let pair = read_call_result(
                &format!("tc-{i}"),
                path,
                &format!("contents v{i}"),
            );
            history.push(pair[0].clone());
            history.push(pair[1].clone());
        }
        history
    }

    fn worker_with_keep_last(keep_last: usize) -> ConsecutiveReadsAutoPruneWorker {
        ConsecutiveReadsAutoPruneWorker {
            config: ConsecutiveReadsAutoPruneConfig {
                enabled: true,
                keep_last,
            },
        }
    }

    fn evaluate(
        worker: &ConsecutiveReadsAutoPruneWorker,
        history: Vec<ChatEntry>,
    ) -> Vec<HistoryMutation> {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            worker
                .evaluate(&SessionId::new(), Arc::from(history))
                .await
        })
    }

    // --- Worker evaluate() tests ---

    #[test]
    fn no_read_calls_produces_no_mutations() {
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::assistant("hi"),
            ChatEntry::user("what is 2+2?"),
            ChatEntry::assistant("4"),
        ];
        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn single_read_produces_no_mutations() {
        let history = history_with_n_reads("/foo.rs", 1);
        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn exact_keep_last_produces_no_mutations() {
        let history = history_with_n_reads("/foo.rs", 3);
        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn one_over_keep_last_prunes_oldest_pair() {
        let history = history_with_n_reads("/foo.rs", 4);
        let expected_call_id = history[0].id.clone();
        let expected_result_id = history[1].id.clone();
        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);

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
    fn two_over_keep_last_prunes_two_oldest_pairs() {
        let history = history_with_n_reads("/foo.rs", 5);
        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);

        // 2 oldest pairs × 2 entries each = 4 mutations.
        assert_eq!(mutations.len(), 4);
    }

    #[test]
    fn multiple_files_pruned_independently() {
        let mut history = Vec::new();

        // 4 reads of /a.rs (exceeds keep_last=2 by 2 → 4 mutations)
        for i in 0..4 {
            let pair = read_call_result(&format!("tc-a-{i}"), "/a.rs", &format!("a v{i}"));
            history.push(pair[0].clone());
            history.push(pair[1].clone());
        }

        // 2 reads of /b.rs (exactly keep_last=2 → 0 mutations)
        for i in 0..2 {
            let pair = read_call_result(&format!("tc-b-{i}"), "/b.rs", &format!("b v{i}"));
            history.push(pair[0].clone());
            history.push(pair[1].clone());
        }

        let worker = worker_with_keep_last(2);
        let mutations = evaluate(&worker, history);

        // 2 pruned pairs for /a.rs × 2 entries each = 4 mutations
        assert_eq!(mutations.len(), 4);
    }

    #[test]
    fn already_excluded_call_and_result_produces_no_duplicate() {
        let mut history = history_with_n_reads("/foo.rs", 4);
        // Mark the oldest pair as already excluded.
        history[0].context_override = ContextOverride::ForcedExclude;
        history[1].context_override = ContextOverride::ForcedExclude;

        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    fn already_excluded_call_only_prunes_result() {
        let mut history = history_with_n_reads("/foo.rs", 4);
        // Mark only the call as excluded.
        history[0].context_override = ContextOverride::ForcedExclude;
        let expected_result_id = history[1].id.clone();

        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);

        // Only the result should get a mutation.
        assert_eq!(mutations.len(), 1);
        match &mutations[0] {
            HistoryMutation::SetContextOverride { entry_id, value } => {
                assert_eq!(*entry_id, expected_result_id);
                assert_eq!(*value, ContextOverride::ForcedExclude);
            }
            other => panic!("expected SetContextOverride, got {other:?}"),
        }
    }

    fn already_excluded_result_only_prunes_call() {
        let mut history = history_with_n_reads("/foo.rs", 4);
        // Mark only the result as excluded.
        history[1].context_override = ContextOverride::ForcedExclude;
        let expected_call_id = history[0].id.clone();

        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);

        // Only the call should get a mutation.
        assert_eq!(mutations.len(), 1);
        match &mutations[0] {
            HistoryMutation::SetContextOverride { entry_id, value } => {
                assert_eq!(*entry_id, expected_call_id);
                assert_eq!(*value, ContextOverride::ForcedExclude);
            }
            other => panic!("expected SetContextOverride, got {other:?}"),
        }
    }

    #[test]
    fn orphaned_call_without_result_is_skipped() {
        let mut history = Vec::new();
        // Read tool call but no corresponding tool result.
        history.push(ChatEntry::tool_call(
            "tc-orphan",
            "read",
            r#"{"path": "/foo.rs"}"#,
        ));

        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn read_without_path_argument_is_skipped() {
        let mut history = Vec::new();
        // ToolCall with name "read" but no "path" in arguments.
        history.push(ChatEntry::tool_call("tc-1", "read", r#"{"offset": 1}"#));
        history.push(ChatEntry::tool_result(
            "tc-1",
            "read",
            "some output",
            ToolResultStatus::Success,
        ));

        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn different_paths_tracked_separately() {
        let mut history = Vec::new();

        let pair1 = read_call_result("tc-1", "/abs/path/foo.rs", "abs contents");
        history.push(pair1[0].clone());
        history.push(pair1[1].clone());

        let pair2 = read_call_result("tc-2", "foo.rs", "rel contents");
        history.push(pair2[0].clone());
        history.push(pair2[1].clone());

        let worker = worker_with_keep_last(1);
        let mutations = evaluate(&worker, history);

        // Each path has 1 pair, keep_last=1, so no pruning.
        assert!(
            mutations.is_empty(),
            "different string paths should be tracked independently"
        );
    }

    #[test]
    fn keep_last_1_prunes_all_but_last() {
        let history = history_with_n_reads("/foo.rs", 5);
        let worker = worker_with_keep_last(1);
        let mutations = evaluate(&worker, history);

        // 4 pruned pairs × 2 entries each = 8 mutations.
        assert_eq!(mutations.len(), 8);
    }

    #[test]
    fn empty_history_produces_no_mutations() {
        let worker = worker_with_keep_last(3);
        let mutations = evaluate(&worker, vec![]);
        assert!(mutations.is_empty());
    }
}
