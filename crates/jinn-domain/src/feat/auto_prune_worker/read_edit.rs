//! Read-edit auto-prune worker.
//!
//! Detects `read` → `edit` tool call patterns on the same file path and marks
//! the read's `ToolResult` as [`ForcedExclude`] once enough in-context entries
//! have accumulated after the edit. This removes stale file contents from the
//! LLM context window.
//!
//! Pruning does not occur until `min_tail_entries` (default: 10) in-context
//! entries have accumulated after the edit.
//!
//! # Example
//!
//! ```text
//!    [User]: show me /foo.rs
//!    [Tool Call]: read(/foo.rs)
//! X  [Tool Result] (OK): <file contents>
//!    [Assistant]: here's what I see...
//!    [User]: now fix the bug
//!    [Tool Call]: edit(/foo.rs)
//!    [Tool Result] (OK): edit applied
//!    [Assistant]: done
//! ```
//!
//! [`ForcedExclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedExclude

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::ReadEditAutoPruneConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

/// Read-edit auto-prune worker.
///
/// Inspects history for `read` tool calls whose results have been superseded
/// by a subsequent `edit` to the same file. Once `min_tail_entries` in-context
/// entries have accumulated after the edit, the read result is excluded.
#[derive(Clone)]
pub struct ReadEditAutoPruneWorker {
    /// Configuration for the read-edit auto-prune strategy.
    pub config: ReadEditAutoPruneConfig,
}

/// Extract the `path` field from a tool call's JSON arguments string.
///
/// Returns `None` if the arguments cannot be parsed or the `path` field is
/// missing or not a string.
fn extract_path_from_arguments(arguments: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    value.get("path")?.as_str().map(std::borrow::ToOwned::to_owned)
}

#[async_trait::async_trait]
impl HistoryWorker for ReadEditAutoPruneWorker {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "auto-prune-read-edit"
    }

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: Vec<ChatEntry>,
    ) -> Vec<HistoryMutation> {
        let mut mutations = Vec::new();

        for i in 0..history.len() {
            let entry = &history[i];

            // Only interested in "read" tool calls.
            let (tool_call_id, read_path) = match &entry.kind {
                ChatEntryKind::ToolCall { name, arguments, id } if name == "read" => {
                    let Some(path) = extract_path_from_arguments(arguments) else {
                        continue;
                    };
                    (id.clone(), path)
                }
                _ => continue,
            };

            // Find the corresponding ToolResult (matched by tool call id).
            let mut result_entry_id = None;
            let mut result_index = None;
            for (j, entry_j) in history.iter().enumerate().skip(i + 1) {
                if let ChatEntryKind::ToolResult { id, .. } = &entry_j.kind
                    && id == &tool_call_id
                {
                    result_entry_id = Some(entry_j.id.clone());
                    result_index = Some(j);
                    break;
                }
            }

            let Some(result_idx) = result_index else {
                continue;
            };

            // Skip if already excluded.
            if history[result_idx].context_override == ContextOverride::ForcedExclude {
                continue;
            }

            // Scan forward for an "edit" on the same file path.
            let mut edit_index = None;
            for (j, entry_j) in history.iter().enumerate().skip(i + 1) {
                if let ChatEntryKind::ToolCall { name, arguments, .. } = &entry_j.kind
                    && name == "edit"
                    && extract_path_from_arguments(arguments).is_some_and(|p| p == read_path)
                {
                    edit_index = Some(j);
                    break;
                }
            }

            let Some(edit_idx) = edit_index else {
                continue;
            };

            // Count in-context entries after the edit.
            let in_context_count = history[(edit_idx + 1)..]
                .iter()
                .filter(|e| e.is_in_context())
                .count();

            if in_context_count >= self.config.min_tail_entries {
                mutations.push(HistoryMutation::SetContextOverride {
                    entry_id: result_entry_id.expect("result_entry_id was set above"),
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
    use crate::feat::preferences_actor::user_preferences::ReadEditAutoPruneConfig;
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

    /// Helper: create an edit ToolCall + ToolResult pair.
    fn edit_call_result(call_id: &str, path: &str, content: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "edit", format!(r#"{{"path": "{path}"}}"#)),
            ChatEntry::tool_result(call_id, "edit", content, ToolResultStatus::Success),
        ]
    }

    /// Build a history with a read→edit pair followed by N user entries (in-context).
    fn history_with_read_edit_and_tail(
        read_path: &str,
        edit_path: &str,
        tail_count: usize,
    ) -> Vec<ChatEntry> {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", read_path, "file contents here");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let edit = edit_call_result("tc-2", edit_path, "edit applied");
        history.push(edit[0].clone());
        history.push(edit[1].clone());
        for i in 0..tail_count {
            history.push(ChatEntry::user(format!("tail message {i}")));
        }
        history
    }

    fn worker_with_tail(tail: usize) -> ReadEditAutoPruneWorker {
        ReadEditAutoPruneWorker {
            config: ReadEditAutoPruneConfig {
                enabled: true,
                min_tail_entries: tail,
            },
        }
    }

    // --- extract_path_from_arguments tests ---

    #[test]
    fn extract_path_from_valid_json() {
        let path = extract_path_from_arguments(r#"{"path": "/foo/bar.rs"}"#);
        assert_eq!(path, Some("/foo/bar.rs".to_owned()));
    }

    #[test]
    fn extract_path_from_json_with_extra_fields() {
        let path = extract_path_from_arguments(r#"{"path": "/foo.rs", "offset": 1, "limit": 50}"#);
        assert_eq!(path, Some("/foo.rs".to_owned()));
    }

    #[test]
    fn extract_path_returns_none_for_missing_path() {
        let path = extract_path_from_arguments(r#"{"file": "/foo.rs"}"#);
        assert_eq!(path, None);
    }

    #[test]
    fn extract_path_returns_none_for_malformed_json() {
        let path = extract_path_from_arguments("not json");
        assert_eq!(path, None);
    }

    #[test]
    fn extract_path_returns_none_for_non_string_path() {
        let path = extract_path_from_arguments(r#"{"path": 42}"#);
        assert_eq!(path, None);
    }

    // --- Worker evaluate() tests ---

    #[test]
    fn no_read_edit_pattern_produces_no_mutations() {
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::assistant("hi"),
            ChatEntry::user("what is 2+2?"),
            ChatEntry::assistant("4"),
        ];
        let worker = worker_with_tail(3);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert!(mutations.is_empty());
    }

    #[test]
    fn read_then_edit_same_file_with_enough_tail_prunes() {
        let history = history_with_read_edit_and_tail("/foo.rs", "/foo.rs", 10);
        let expected_result_id = history[1].id.clone();
        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert_eq!(mutations.len(), 1);
        // The mutation should target the read ToolResult.
        match &mutations[0] {
            HistoryMutation::SetContextOverride { entry_id, value } => {
                assert_eq!(*entry_id, expected_result_id);
                assert_eq!(*value, ContextOverride::ForcedExclude);
            }
            other => panic!("expected SetContextOverride, got {other:?}"),
        }
    }

    #[test]
    fn read_then_edit_different_files_does_not_prune() {
        let history = history_with_read_edit_and_tail("/foo.rs", "/bar.rs", 10);
        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert!(mutations.is_empty());
    }

    #[test]
    fn tail_below_threshold_does_not_prune() {
        let history = history_with_read_edit_and_tail("/foo.rs", "/foo.rs", 5);
        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert!(mutations.is_empty());
    }

    #[test]
    fn already_excluded_produces_no_duplicate_mutation() {
        let mut history = history_with_read_edit_and_tail("/foo.rs", "/foo.rs", 10);
        // Mark the read result as already excluded.
        history[1].context_override = ContextOverride::ForcedExclude;

        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert!(mutations.is_empty());
    }

    #[test]
    fn multiple_read_edit_pairs_both_pruned() {
        let mut history = Vec::new();

        // First pair: read /a.rs → edit /a.rs
        let read_a = read_call_result("tc-1", "/a.rs", "contents of a");
        history.push(read_a[0].clone());
        history.push(read_a[1].clone());
        let edit_a = edit_call_result("tc-2", "/a.rs", "edit a");
        history.push(edit_a[0].clone());
        history.push(edit_a[1].clone());

        // Second pair: read /b.rs → edit /b.rs
        let read_b = read_call_result("tc-3", "/b.rs", "contents of b");
        history.push(read_b[0].clone());
        history.push(read_b[1].clone());
        let edit_b = edit_call_result("tc-4", "/b.rs", "edit b");
        history.push(edit_b[0].clone());
        history.push(edit_b[1].clone());

        // Add tail entries after last edit.
        for i in 0..10 {
            history.push(ChatEntry::user(format!("tail {i}")));
        }

        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert_eq!(mutations.len(), 2);
    }

    #[test]
    fn read_without_result_produces_no_mutation() {
        let mut history = Vec::new();
        // Read tool call but no corresponding tool result.
        history.push(ChatEntry::tool_call("tc-orphan", "read", r#"{"path": "/foo.rs"}"#));
        // Some edit on same file.
        let edit = edit_call_result("tc-2", "/foo.rs", "edit done");
        history.push(edit[0].clone());
        history.push(edit[1].clone());
        for i in 0..10 {
            history.push(ChatEntry::user(format!("tail {i}")));
        }

        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert!(mutations.is_empty());
    }

    #[test]
    fn edit_without_prior_read_produces_no_mutation() {
        let mut history = Vec::new();
        let edit = edit_call_result("tc-1", "/foo.rs", "edit done");
        history.push(edit[0].clone());
        history.push(edit[1].clone());
        for i in 0..10 {
            history.push(ChatEntry::user(format!("tail {i}")));
        }

        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert!(mutations.is_empty());
    }

    #[test]
    fn exact_threshold_prunes() {
        // Exactly min_tail_entries should prune.
        let history = history_with_read_edit_and_tail("/foo.rs", "/foo.rs", 10);
        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert_eq!(mutations.len(), 1);
    }

    #[test]
    fn one_below_threshold_does_not_prune() {
        // min_tail_entries=10. History: read_call, read_result, edit_call, edit_result, 8 users.
        // In-context after edit: edit_result (1) + 8 users = 9 < 10.
        let history = history_with_read_edit_and_tail("/foo.rs", "/foo.rs", 8);
        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert!(mutations.is_empty());
    }


    #[test]
    fn only_counts_in_context_entries() {
        // Create history where many entries follow the edit but few are in context.
        // After edit: edit_result (in-context) + 8 user (in-context) + 6 transient (not in-context).
        // In-context count = 9 < 10, so no prune.
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let edit = edit_call_result("tc-2", "/foo.rs", "edited");
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
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert!(mutations.is_empty(), "should not prune: only 9 in-context entries");
    }

    #[test]
    fn same_file_read_twice_edited_once_both_pruned() {
        let mut history = Vec::new();

        // First read of /foo.rs
        let read1 = read_call_result("tc-1", "/foo.rs", "contents v1");
        history.push(read1[0].clone());
        history.push(read1[1].clone());

        // Second read of /foo.rs
        let read2 = read_call_result("tc-2", "/foo.rs", "contents v2");
        history.push(read2[0].clone());
        history.push(read2[1].clone());

        // Edit /foo.rs
        let edit = edit_call_result("tc-3", "/foo.rs", "edited");
        history.push(edit[0].clone());
        history.push(edit[1].clone());

        for i in 0..10 {
            history.push(ChatEntry::user(format!("tail {i}")));
        }

        let worker = worker_with_tail(10);
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mutations = rt.block_on(async {
            worker.evaluate(&SessionId::new(), history).await
        });
        assert_eq!(mutations.len(), 2, "both reads should be pruned");
    }
}
