//! Read-edit auto-prune worker.
//!
//! Detects `read` → `edit` tool call patterns on the same file path and marks
//! both the read `ToolCall` and `ToolResult` as [`ForcedExclude`] once
//! [`EDIT_THRESHOLD`] (2) subsequent edits to the same file have occurred.
//! This removes stale file contents from the LLM context window.
//!
//! Pruning is immediate — no tail-entry delay.
//!
//! # Example
//!
//! ```text
//! X  [Tool Call]: read(/foo.rs)
//! X  [Tool Result] (OK): <file contents>
//!    [Tool Call]: edit(/foo.rs)
//!    [Tool Result] (OK): edit applied
//!    [Tool Call]: edit(/foo.rs)
//!    [Tool Result] (OK): edit applied
//!    [Assistant]: done
//! ```
//!
//! [`ForcedExclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedExclude

use std::sync::Arc;

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::ReadEditAutoPruneConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

/// Number of edits on the same file required before pruning the prior read.
const EDIT_THRESHOLD: usize = 2;

/// Read-edit auto-prune worker.
///
/// Inspects history for `read` tool calls whose contents have been superseded
/// by subsequent `edit` calls to the same file. Once [`EDIT_THRESHOLD`] edits
/// have occurred after a read, both the read ToolCall and its ToolResult are
/// excluded from context.
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
    value
        .get("path")?
        .as_str()
        .map(std::borrow::ToOwned::to_owned)
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
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        let mut mutations = Vec::new();

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

            let read_call_entry_id = entry.id.clone();
            let call_already_excluded = entry.context_override == ContextOverride::ForcedExclude;

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

            let result_already_excluded =
                history[result_idx].context_override == ContextOverride::ForcedExclude;

            // Skip if both are already excluded.
            if call_already_excluded && result_already_excluded {
                continue;
            }

            // Walk forward counting edits on the same file path.
            let mut edit_count: usize = 0;
            for entry_j in history.iter().skip(i + 1) {
                if let ChatEntryKind::ToolCall {
                    name, arguments, ..
                } = &entry_j.kind
                    && name == "edit"
                    && extract_path_from_arguments(arguments).is_some_and(|p| p == read_path)
                {
                    edit_count += 1;
                    if edit_count >= EDIT_THRESHOLD {
                        break;
                    }
                }
            }

            if edit_count >= EDIT_THRESHOLD {
                if !call_already_excluded {
                    mutations.push(HistoryMutation::SetContextOverride {
                        entry_id: read_call_entry_id,
                        value: ContextOverride::ForcedExclude,
                    });
                }
                if !result_already_excluded {
                    mutations.push(HistoryMutation::SetContextOverride {
                        entry_id: result_entry_id.expect("result_entry_id was set above"),
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

    fn worker() -> ReadEditAutoPruneWorker {
        ReadEditAutoPruneWorker {
            config: ReadEditAutoPruneConfig { enabled: true },
        }
    }

    /// Evaluate the worker synchronously for tests.
    fn evaluate(history: Vec<ChatEntry>) -> Vec<HistoryMutation> {
        let w = worker();
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async { w.evaluate(&SessionId::new(), Arc::from(history)).await })
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
        let mutations = evaluate(history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn read_then_one_edit_no_prune() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "file contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let edit = edit_call_result("tc-2", "/foo.rs", "edit applied");
        history.push(edit[0].clone());
        history.push(edit[1].clone());

        let mutations = evaluate(history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn read_then_two_edits_prunes_both_call_and_result() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "file contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let edit1 = edit_call_result("tc-2", "/foo.rs", "edit 1 applied");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let edit2 = edit_call_result("tc-3", "/foo.rs", "edit 2 applied");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());

        let mutations = evaluate(history);
        assert_eq!(mutations.len(), 2, "should prune both call and result");

        let mutation_ids: Vec<_> = mutations
            .iter()
            .filter_map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, value } => {
                    assert_eq!(*value, ContextOverride::ForcedExclude);
                    Some(entry_id.clone())
                }
                _ => None,
            })
            .collect();

        assert!(
            mutation_ids.contains(&read[0].id),
            "read ToolCall should be pruned"
        );
        assert!(
            mutation_ids.contains(&read[1].id),
            "read ToolResult should be pruned"
        );
    }

    #[test]
    fn read_edit_different_files_no_prune() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let edit1 = edit_call_result("tc-2", "/bar.rs", "edit 1");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let edit2 = edit_call_result("tc-3", "/bar.rs", "edit 2");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());

        let mutations = evaluate(history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn already_excluded_call_no_duplicate_mutation() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        let mut call = read[0].clone();
        call.context_override = ContextOverride::ForcedExclude;
        history.push(call);
        history.push(read[1].clone()); // result NOT excluded
        let edit1 = edit_call_result("tc-2", "/foo.rs", "edit 1");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let edit2 = edit_call_result("tc-3", "/foo.rs", "edit 2");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());

        let mutations = evaluate(history);
        assert_eq!(mutations.len(), 1, "only result should be pruned");

        match &mutations[0] {
            HistoryMutation::SetContextOverride { entry_id, value } => {
                assert_eq!(*entry_id, read[1].id, "should target read ToolResult");
                assert_eq!(*value, ContextOverride::ForcedExclude);
            }
            other => panic!("expected SetContextOverride, got {other:?}"),
        }
    }

    #[test]
    fn already_excluded_result_no_duplicate_mutation() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        history.push(read[0].clone()); // call NOT excluded
        let mut result = read[1].clone();
        result.context_override = ContextOverride::ForcedExclude;
        history.push(result);
        let edit1 = edit_call_result("tc-2", "/foo.rs", "edit 1");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let edit2 = edit_call_result("tc-3", "/foo.rs", "edit 2");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());

        let mutations = evaluate(history);
        assert_eq!(mutations.len(), 1, "only call should be pruned");

        match &mutations[0] {
            HistoryMutation::SetContextOverride { entry_id, value } => {
                assert_eq!(*entry_id, read[0].id, "should target read ToolCall");
                assert_eq!(*value, ContextOverride::ForcedExclude);
            }
            other => panic!("expected SetContextOverride, got {other:?}"),
        }
    }

    #[test]
    fn both_already_excluded_no_mutations() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        let mut call = read[0].clone();
        call.context_override = ContextOverride::ForcedExclude;
        let mut result = read[1].clone();
        result.context_override = ContextOverride::ForcedExclude;
        history.push(call);
        history.push(result);
        let edit1 = edit_call_result("tc-2", "/foo.rs", "edit 1");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let edit2 = edit_call_result("tc-3", "/foo.rs", "edit 2");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());

        let mutations = evaluate(history);
        assert!(
            mutations.is_empty(),
            "should not produce mutations for already-excluded entries"
        );
    }

    #[test]
    fn multiple_reads_same_file_both_pruned() {
        let mut history = Vec::new();
        // First read of /foo.rs
        let read1 = read_call_result("tc-1", "/foo.rs", "contents v1");
        history.push(read1[0].clone());
        history.push(read1[1].clone());
        // Second read of /foo.rs
        let read2 = read_call_result("tc-2", "/foo.rs", "contents v2");
        history.push(read2[0].clone());
        history.push(read2[1].clone());
        // Two edits of /foo.rs
        let edit1 = edit_call_result("tc-3", "/foo.rs", "edit 1");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let edit2 = edit_call_result("tc-4", "/foo.rs", "edit 2");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());

        let mutations = evaluate(history);
        assert_eq!(mutations.len(), 4, "both reads should be pruned (call+result each)");

        let mutation_ids: Vec<_> = mutations
            .iter()
            .filter_map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, .. } => Some(entry_id.clone()),
                _ => None,
            })
            .collect();

        assert!(mutation_ids.contains(&read1[0].id));
        assert!(mutation_ids.contains(&read1[1].id));
        assert!(mutation_ids.contains(&read2[0].id));
        assert!(mutation_ids.contains(&read2[1].id));
    }

    #[test]
    fn interleaved_files_tracked_independently() {
        let mut history = Vec::new();
        let read_a = read_call_result("tc-1", "/a.rs", "contents a");
        history.push(read_a[0].clone());
        history.push(read_a[1].clone());
        let read_b = read_call_result("tc-2", "/b.rs", "contents b");
        history.push(read_b[0].clone());
        history.push(read_b[1].clone());
        let edit_a1 = edit_call_result("tc-3", "/a.rs", "edit a1");
        history.push(edit_a1[0].clone());
        history.push(edit_a1[1].clone());
        let edit_b1 = edit_call_result("tc-4", "/b.rs", "edit b1");
        history.push(edit_b1[0].clone());
        history.push(edit_b1[1].clone());
        let edit_a2 = edit_call_result("tc-5", "/a.rs", "edit a2");
        history.push(edit_a2[0].clone());
        history.push(edit_a2[1].clone());
        let edit_b2 = edit_call_result("tc-6", "/b.rs", "edit b2");
        history.push(edit_b2[0].clone());
        history.push(edit_b2[1].clone());

        let mutations = evaluate(history);
        assert_eq!(mutations.len(), 4, "both reads should be pruned");

        let mutation_ids: Vec<_> = mutations
            .iter()
            .filter_map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, .. } => Some(entry_id.clone()),
                _ => None,
            })
            .collect();

        assert!(mutation_ids.contains(&read_a[0].id));
        assert!(mutation_ids.contains(&read_a[1].id));
        assert!(mutation_ids.contains(&read_b[0].id));
        assert!(mutation_ids.contains(&read_b[1].id));
    }

    #[test]
    fn orphan_read_no_result_no_mutation() {
        let mut history = Vec::new();
        // Read tool call but no corresponding tool result.
        history.push(ChatEntry::tool_call(
            "tc-orphan",
            "read",
            r#"{"path": "/foo.rs"}"#,
        ));
        let edit1 = edit_call_result("tc-2", "/foo.rs", "edit 1");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let edit2 = edit_call_result("tc-3", "/foo.rs", "edit 2");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());

        let mutations = evaluate(history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn edit_before_read_not_counted() {
        let mut history = Vec::new();
        // Edit before the read — should not count.
        let edit0 = edit_call_result("tc-0", "/foo.rs", "edit 0");
        history.push(edit0[0].clone());
        history.push(edit0[1].clone());
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        // Only 1 edit after the read.
        let edit1 = edit_call_result("tc-2", "/foo.rs", "edit 1");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());

        let mutations = evaluate(history);
        assert!(mutations.is_empty(), "only 1 edit after the read, should not prune");
    }

    #[test]
    fn three_edits_still_prunes() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let edit1 = edit_call_result("tc-2", "/foo.rs", "edit 1");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let edit2 = edit_call_result("tc-3", "/foo.rs", "edit 2");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());
        let edit3 = edit_call_result("tc-4", "/foo.rs", "edit 3");
        history.push(edit3[0].clone());
        history.push(edit3[1].clone());

        let mutations = evaluate(history);
        assert_eq!(mutations.len(), 2, "should still prune with 3 edits");
    }
}
