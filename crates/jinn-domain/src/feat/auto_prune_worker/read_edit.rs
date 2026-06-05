//! Read-edit-write auto-prune worker.
//!
//! Two independent pruning strategies triggered by `read` tool calls:
//!
//! 1. **Backward pruning** — When a file is read, all prior `edit` and `write`
//!    ToolCall+ToolResult pairs on the same file are marked [`ForcedExclude`].
//!    The read output now represents the current file state, making prior
//!    edits/writes stale.
//!
//! 2. **Forward pruning** — After a read, once [`WRITE_THRESHOLD`] (2) subsequent
//!    `edit` or `write` calls to the same file have occurred, the read
//!    ToolCall+ToolResult are marked [`ForcedExclude`]. The read contents are
//!    now guaranteed stale.
//!
//! Both strategies are immediate — no tail-entry delay.
//!
//! # Example
//!
//! ```text
//! Backward:
//!   X  [Tool Call]: edit(/foo.rs)        ← pruned (stale before read)
//!   X  [Tool Result] (OK): edit applied   ← pruned
//!      [Tool Call]: read(/foo.rs)          ← triggers backward scan
//!      [Tool Result] (OK): <contents>
//!
//! Forward:
//!      [Tool Call]: read(/foo.rs)          ← triggers forward count
//!      [Tool Result] (OK): <contents>
//!      [Tool Call]: edit(/foo.rs)
//!      [Tool Result] (OK): edit applied
//!   X  [Tool Call]: write(/foo.rs)        ← 2nd edit/write → prunes read
//!   X  [Tool Result] (OK): written        ← (this edit/write is NOT pruned)
//!   X  [Tool Call]: read(/foo.rs)          ← now pruned (stale)
//!   X  [Tool Result] (OK): <contents>     ← now pruned
//! ```
//!
//! [`ForcedExclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedExclude

use std::sync::Arc;

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::ReadEditAutoPruneConfig;
use crate::feat::session::chat_entry::{ChangeSource, ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

/// Number of edit/write operations on the same file required before pruning the prior read.
const WRITE_THRESHOLD: usize = 2;

/// Tool names that modify files and count toward the forward-pruning threshold.
const MODIFY_TOOLS: &[&str] = &["edit", "write"];

/// Returns true if the tool name is a file-modifying tool (edit or write).
fn is_modify_tool(name: &str) -> bool {
    MODIFY_TOOLS.contains(&name)
}

/// Read-edit-write auto-prune worker.
///
/// For each `read` tool call in history:
/// - **Backward**: prunes all prior `edit`/`write` ToolCall+ToolResult pairs
///   on the same file.
/// - **Forward**: once [`WRITE_THRESHOLD`] subsequent `edit`/`write` calls
///   on the same file have occurred, prunes the read ToolCall+ToolResult.
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

/// Walk forward from a ToolCall at `call_idx` to find its matching ToolResult.
///
/// Returns `Some((result_entry_id, result_index))` if a match is found.
/// Returns `None` if no result exists (pending/orphaned).
fn find_matching_result(
    history: &[ChatEntry],
    call_idx: usize,
    tool_call_id: &str,
) -> Option<(crate::feat::session::chat_entry::ChatEntryId, usize)> {
    // ToolResults appear after their ToolCall, so scan forward only.
    for (j, entry) in history.iter().enumerate().skip(call_idx + 1) {
        if let ChatEntryKind::ToolResult { id, .. } = &entry.kind
            && id == tool_call_id
        {
            return Some((entry.id.clone(), j));
        }
    }
    // No matching result found — the call is still pending or orphaned.
    None
}

/// Walk backward from a read tool call at `read_index` and prune all
/// edit/write ToolCall+ToolResult pairs on the same file path.
///
/// Runs regardless of whether the read itself is excluded — stale edits are
/// noise even if the read is already pruned.
fn prune_backward(
    history: &[ChatEntry],
    read_index: usize,
    read_path: &str,
    mutations: &mut Vec<HistoryMutation>,
    worker_name: &str,
) {
    // Walk backward from the read to find prior edit/write calls on the same file.
    for j in (0..read_index).rev() {
        let back_entry = &history[j];

        // Only interested in edit/write tool calls targeting the same file path.
        let back_call_id = match &back_entry.kind {
            ChatEntryKind::ToolCall {
                name,
                arguments,
                id,
            } if is_modify_tool(name)
                && extract_path_from_arguments(arguments).is_some_and(|p| p == read_path) =>
            {
                id.clone()
            }
            _ => continue,
        };

        let back_call_entry_id = back_entry.id.clone();
        let back_call_excluded = back_entry.context_override() == ContextOverride::ForcedExclude;

        // Walk forward from this edit/write call to find its matching ToolResult.
        // The result may appear anywhere after the call (not necessarily right after).
        let back_result = find_matching_result(history, j, &back_call_id);

        let back_result_excluded = back_result
            .as_ref()
            .is_some_and(|(_, k)| history[*k].context_override() == ContextOverride::ForcedExclude);

        // Skip if both call and result are already excluded — nothing to do.
        if back_call_excluded && back_result_excluded {
            continue;
        }

        if !back_call_excluded {
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: back_call_entry_id,
                value: ContextOverride::ForcedExclude,
                source: ChangeSource::Worker {
                    name: worker_name.to_owned(),
                },
            });
        }
        if let Some((result_id, _)) = back_result.filter(|_| !back_result_excluded) {
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: result_id,
                value: ContextOverride::ForcedExclude,
                source: ChangeSource::Worker {
                    name: worker_name.to_owned(),
                },
            });
        }
    }
}

/// Count how many edit/write ToolCalls on the same file path appear after
/// the given index. Stops early once the count reaches `threshold`.
///
/// This determines whether the read's contents are stale — once enough
/// modifications have happened after the read, the read output no longer
/// represents the file's current state.
fn count_subsequent_modifications(
    history: &[ChatEntry],
    after_idx: usize,
    file_path: &str,
    threshold: usize,
) -> usize {
    let mut count: usize = 0;
    for entry in history.iter().skip(after_idx + 1) {
        if let ChatEntryKind::ToolCall {
            name, arguments, ..
        } = &entry.kind
            && is_modify_tool(name)
            && extract_path_from_arguments(arguments).is_some_and(|p| p == file_path)
        {
            count += 1;
            if count >= threshold {
                break;
            }
        }
    }
    count
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

            // Only interested in "read" tool calls with a parseable file path.
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
            let call_already_excluded = entry.context_override() == ContextOverride::ForcedExclude;

            // ── Backward pruning ──────────────────────────────────────────
            // Walk backward from the read and prune all edit/write call+result
            // pairs on the same file. Runs regardless of the read's exclusion
            // state — stale edits are noise even if the read itself is excluded.
            prune_backward(&history, i, &read_path, &mut mutations, self.name());

            // ── Forward pruning ──────────────────────────────────────────
            // Find the read's corresponding ToolResult. If none found,
            // skip forward pruning — an orphaned read has no pair to prune.
            let Some((result_entry_id, result_idx)) =
                find_matching_result(&history, i, &tool_call_id)
            else {
                continue;
            };

            let result_already_excluded =
                history[result_idx].context_override() == ContextOverride::ForcedExclude;

            // Skip forward pruning if both call and result are already excluded.
            if call_already_excluded && result_already_excluded {
                continue;
            }

            // Count how many edit/write calls to the same file appear after
            // this read. Once the threshold is reached, the read is stale.
            let modify_count =
                count_subsequent_modifications(&history, i, &read_path, WRITE_THRESHOLD);

            if modify_count >= WRITE_THRESHOLD {
                if !call_already_excluded {
                    mutations.push(HistoryMutation::SetContextOverride {
                        entry_id: read_call_entry_id,
                        value: ContextOverride::ForcedExclude,
                        source: ChangeSource::Worker {
                            name: self.name().to_owned(),
                        },
                    });
                }
                if !result_already_excluded {
                    mutations.push(HistoryMutation::SetContextOverride {
                        entry_id: result_entry_id,
                        value: ContextOverride::ForcedExclude,
                        source: ChangeSource::Worker {
                            name: self.name().to_owned(),
                        },
                    });
                }
            }
        }

        mutations
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::similar_names,
        reason = "test code"
    )]

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

    /// Helper: create a write ToolCall + ToolResult pair.
    fn write_call_result(call_id: &str, path: &str, content: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "write", format!(r#"{{"path": "{path}"}}"#)),
            ChatEntry::tool_result(call_id, "write", content, ToolResultStatus::Success),
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

    /// Collect mutation entry IDs from a list of mutations.
    fn mutation_ids(
        mutations: &[HistoryMutation],
    ) -> Vec<crate::feat::session::chat_entry::ChatEntryId> {
        mutations
            .iter()
            .filter_map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, value, .. } => {
                    assert_eq!(*value, ContextOverride::ForcedExclude);
                    Some(entry_id.clone())
                }
                _ => None,
            })
            .collect()
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

    // --- is_modify_tool tests ---

    #[test]
    fn is_modify_tool_recognizes_edit_and_write() {
        assert!(is_modify_tool("edit"));
        assert!(is_modify_tool("write"));
        assert!(!is_modify_tool("read"));
        assert!(!is_modify_tool("bash"));
    }

    // --- No-read baseline tests ---

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

    // --- Forward pruning tests (edit-only, existing behavior) ---

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

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&read[0].id), "read ToolCall should be pruned");
        assert!(
            ids.contains(&read[1].id),
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
        call.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() });
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
            HistoryMutation::SetContextOverride { entry_id, value, .. } => {
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
        result.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() });
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
            HistoryMutation::SetContextOverride { entry_id, value, .. } => {
                assert_eq!(*entry_id, read[0].id, "should target read ToolCall");
                assert_eq!(*value, ContextOverride::ForcedExclude);
            }
            other => panic!("expected SetContextOverride, got {other:?}"),
        }
    }

    #[test]
    fn both_already_excluded_no_forward_mutations() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        let mut call = read[0].clone();
        call.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() });
        let mut result = read[1].clone();
        result.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() });
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
            "should not produce forward-pruning mutations for already-excluded entries"
        );
    }

    #[test]
    fn multiple_reads_same_file_both_forward_pruned() {
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
        // 2 mutations per read (call+result) × 2 reads = 4 forward-pruning mutations.
        // No backward pruning because no edits precede any read.
        assert_eq!(
            mutations.len(),
            4,
            "both reads should be pruned (call+result each)"
        );

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&read1[0].id));
        assert!(ids.contains(&read1[1].id));
        assert!(ids.contains(&read2[0].id));
        assert!(ids.contains(&read2[1].id));
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

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&read_a[0].id));
        assert!(ids.contains(&read_a[1].id));
        assert!(ids.contains(&read_b[0].id));
        assert!(ids.contains(&read_b[1].id));
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
    fn edit_before_read_not_counted_forward() {
        let mut history = Vec::new();
        // Edit before the read — should not count toward forward threshold.
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
        // The read should NOT be forward-pruned (only 1 edit after it).
        // But edit0 should be backward-pruned (edit before read).
        let ids = mutation_ids(&mutations);
        assert!(
            !ids.contains(&read[0].id),
            "read should not be forward-pruned with only 1 subsequent edit"
        );
        assert!(
            !ids.contains(&read[1].id),
            "read result should not be forward-pruned with only 1 subsequent edit"
        );
        assert!(
            ids.contains(&edit0[0].id),
            "edit0 call should be backward-pruned"
        );
        assert!(
            ids.contains(&edit0[1].id),
            "edit0 result should be backward-pruned"
        );
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
        assert_eq!(mutations.len(), 2, "should still prune read with 3 edits");
    }

    // --- Forward pruning with write support ---

    #[test]
    fn read_then_two_writes_prunes_read() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let write1 = write_call_result("tc-2", "/foo.rs", "written 1");
        history.push(write1[0].clone());
        history.push(write1[1].clone());
        let write2 = write_call_result("tc-3", "/foo.rs", "written 2");
        history.push(write2[0].clone());
        history.push(write2[1].clone());

        let mutations = evaluate(history);
        assert_eq!(mutations.len(), 2, "should prune read after 2 writes");

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&read[0].id));
        assert!(ids.contains(&read[1].id));
    }

    #[test]
    fn read_then_mixed_edit_and_write_prunes_read() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let edit1 = edit_call_result("tc-2", "/foo.rs", "edit applied");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let write1 = write_call_result("tc-3", "/foo.rs", "written");
        history.push(write1[0].clone());
        history.push(write1[1].clone());

        let mutations = evaluate(history);
        assert_eq!(mutations.len(), 2, "1 edit + 1 write = threshold met");

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&read[0].id));
        assert!(ids.contains(&read[1].id));
    }

    #[test]
    fn read_then_one_write_no_prune() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let write1 = write_call_result("tc-2", "/foo.rs", "written");
        history.push(write1[0].clone());
        history.push(write1[1].clone());

        let mutations = evaluate(history);
        assert!(mutations.is_empty());
    }

    // --- Backward pruning tests ---

    #[test]
    fn backward_prunes_prior_edits_on_same_file() {
        let mut history = Vec::new();
        let edit1 = edit_call_result("tc-1", "/foo.rs", "edit 1 applied");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let edit2 = edit_call_result("tc-2", "/foo.rs", "edit 2 applied");
        history.push(edit2[0].clone());
        history.push(edit2[1].clone());
        let read = read_call_result("tc-3", "/foo.rs", "file contents");
        history.push(read[0].clone());
        history.push(read[1].clone());

        let mutations = evaluate(history);
        // edit1 call+result + edit2 call+result = 4 backward mutations.
        assert_eq!(
            mutations.len(),
            4,
            "both prior edits should be backward-pruned"
        );

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&edit1[0].id));
        assert!(ids.contains(&edit1[1].id));
        assert!(ids.contains(&edit2[0].id));
        assert!(ids.contains(&edit2[1].id));
    }

    #[test]
    fn backward_prunes_prior_writes_on_same_file() {
        let mut history = Vec::new();
        let write1 = write_call_result("tc-1", "/foo.rs", "written 1");
        history.push(write1[0].clone());
        history.push(write1[1].clone());
        let write2 = write_call_result("tc-2", "/foo.rs", "written 2");
        history.push(write2[0].clone());
        history.push(write2[1].clone());
        let read = read_call_result("tc-3", "/foo.rs", "file contents");
        history.push(read[0].clone());
        history.push(read[1].clone());

        let mutations = evaluate(history);
        assert_eq!(
            mutations.len(),
            4,
            "both prior writes should be backward-pruned"
        );

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&write1[0].id));
        assert!(ids.contains(&write1[1].id));
        assert!(ids.contains(&write2[0].id));
        assert!(ids.contains(&write2[1].id));
    }

    #[test]
    fn backward_prunes_mixed_prior_edits_and_writes() {
        let mut history = Vec::new();
        let edit1 = edit_call_result("tc-1", "/foo.rs", "edit applied");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let write1 = write_call_result("tc-2", "/foo.rs", "written");
        history.push(write1[0].clone());
        history.push(write1[1].clone());
        let read = read_call_result("tc-3", "/foo.rs", "file contents");
        history.push(read[0].clone());
        history.push(read[1].clone());

        let mutations = evaluate(history);
        assert_eq!(
            mutations.len(),
            4,
            "prior edit and write should both be backward-pruned"
        );

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&edit1[0].id));
        assert!(ids.contains(&edit1[1].id));
        assert!(ids.contains(&write1[0].id));
        assert!(ids.contains(&write1[1].id));
    }

    #[test]
    fn backward_no_mutation_when_no_prior_edits_or_writes() {
        let mut history = Vec::new();
        let read = read_call_result("tc-1", "/foo.rs", "file contents");
        history.push(read[0].clone());
        history.push(read[1].clone());

        let mutations = evaluate(history);
        assert!(mutations.is_empty(), "nothing to backward-prune");
    }

    #[test]
    fn backward_does_not_prune_different_files() {
        let mut history = Vec::new();
        let edit1 = edit_call_result("tc-1", "/bar.rs", "edit on bar");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let read = read_call_result("tc-2", "/foo.rs", "foo contents");
        history.push(read[0].clone());
        history.push(read[1].clone());

        let mutations = evaluate(history);
        assert!(
            mutations.is_empty(),
            "edit on different file should not be pruned"
        );
    }

    #[test]
    fn backward_already_excluded_edit_no_duplicate() {
        let mut history = Vec::new();
        let edit1 = edit_call_result("tc-1", "/foo.rs", "edit applied");
        let mut edit_call = edit1[0].clone();
        edit_call.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() });
        let mut edit_result = edit1[1].clone();
        edit_result.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() });
        history.push(edit_call);
        history.push(edit_result);
        let read = read_call_result("tc-2", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());

        let mutations = evaluate(history);
        assert!(
            mutations.is_empty(),
            "already-excluded edit should not produce duplicate mutations"
        );
    }

    #[test]
    fn backward_runs_even_when_read_already_excluded() {
        let mut history = Vec::new();
        let edit1 = edit_call_result("tc-1", "/foo.rs", "edit applied");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let read = read_call_result("tc-2", "/foo.rs", "contents");
        let mut read_call = read[0].clone();
        read_call.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() });
        let mut read_result = read[1].clone();
        read_result.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() });
        history.push(read_call);
        history.push(read_result);

        let mutations = evaluate(history);
        // Even though the read is fully excluded, backward pruning should still run.
        assert_eq!(
            mutations.len(),
            2,
            "prior edit should still be backward-pruned"
        );

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&edit1[0].id));
        assert!(ids.contains(&edit1[1].id));
    }

    #[test]
    fn backward_pruning_independent_of_forward() {
        // Read with 0 subsequent edits — forward pruning won't trigger,
        // but backward pruning should still prune prior edits.
        let mut history = Vec::new();
        let edit1 = edit_call_result("tc-1", "/foo.rs", "edit applied");
        history.push(edit1[0].clone());
        history.push(edit1[1].clone());
        let read = read_call_result("tc-2", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        // No edits after the read.

        let mutations = evaluate(history);
        // edit1 should be backward-pruned (2 mutations).
        // read should NOT be forward-pruned (0 edits after it).
        assert_eq!(mutations.len(), 2);

        let ids = mutation_ids(&mutations);
        assert!(ids.contains(&edit1[0].id));
        assert!(ids.contains(&edit1[1].id));
        assert!(
            !ids.contains(&read[0].id),
            "read should not be forward-pruned"
        );
        assert!(
            !ids.contains(&read[1].id),
            "read result should not be forward-pruned"
        );
    }

    #[test]
    fn backward_skips_edit_without_result() {
        let mut history = Vec::new();
        let orphan_entry = ChatEntry::tool_call("tc-orphan", "edit", r#"{"path": "/foo.rs"}"#);
        let orphan_id = orphan_entry.id.clone();
        history.push(orphan_entry);
        let read = read_call_result("tc-2", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());

        let mutations = evaluate(history);
        // The orphan edit call has no result to find. The call itself should be pruned.
        assert_eq!(
            mutations.len(),
            1,
            "orphan edit call should still be pruned"
        );

        let ids = mutation_ids(&mutations);
        assert!(
            ids.contains(&orphan_id),
            "orphan edit call should be pruned"
        );
    }

    // --- Combined backward + forward tests ---

    #[test]
    fn both_backward_and_forward_fire_together() {
        let mut history = Vec::new();
        let edit_before = edit_call_result("tc-1", "/foo.rs", "edit before");
        history.push(edit_before[0].clone());
        history.push(edit_before[1].clone());
        let read = read_call_result("tc-2", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());
        let edit_after1 = edit_call_result("tc-3", "/foo.rs", "edit after 1");
        history.push(edit_after1[0].clone());
        history.push(edit_after1[1].clone());
        let edit_after2 = edit_call_result("tc-4", "/foo.rs", "edit after 2");
        history.push(edit_after2[0].clone());
        history.push(edit_after2[1].clone());

        let mutations = evaluate(history);
        // Backward: edit_before call+result = 2 mutations.
        // Forward: read call+result = 2 mutations.
        assert_eq!(mutations.len(), 4);

        let ids = mutation_ids(&mutations);
        assert!(
            ids.contains(&edit_before[0].id),
            "backward: edit_before call"
        );
        assert!(
            ids.contains(&edit_before[1].id),
            "backward: edit_before result"
        );
        assert!(ids.contains(&read[0].id), "forward: read call");
        assert!(ids.contains(&read[1].id), "forward: read result");
    }

    #[test]
    fn backward_prunes_five_prior_edits() {
        let mut history = Vec::new();
        let mut prior_edits = Vec::new();
        for i in 0..5 {
            let edit = edit_call_result(&format!("tc-{i}"), "/foo.rs", &format!("edit {i}"));
            history.push(edit[0].clone());
            history.push(edit[1].clone());
            prior_edits.push(edit);
        }
        let read = read_call_result("tc-read", "/foo.rs", "contents");
        history.push(read[0].clone());
        history.push(read[1].clone());

        let mutations = evaluate(history);
        // 5 edits × 2 (call+result) = 10 backward mutations.
        assert_eq!(mutations.len(), 10);

        let ids = mutation_ids(&mutations);
        for edit in &prior_edits {
            assert!(ids.contains(&edit[0].id), "edit call should be pruned");
            assert!(ids.contains(&edit[1].id), "edit result should be pruned");
        }
    }
}
