//! Todo auto-prune worker.
//!
//! Detects all `todo_`-prefixed tool calls in conversation history and treats
//! them as one unified group. Keeps only the most recent `ToolCall` +
//! `ToolResult` pair and marks all older pairs as [`ForcedExclude`].
//! This removes stale todo state from the LLM context window.
//!
//! Pruning is immediate — no threshold or delay.
//!
//! # Example
//!
//! ```text
//! X  [Tool Call]: todo_add_task("Write code")
//! X  [Tool Result] (OK): <stale task list>
//! X  [Tool Call]: todo_complete_task("t1")
//! X  [Tool Result] (OK): <stale task list>
//!    [Tool Call]: todo_add_phase("Test")
//!    [Tool Result] (OK): <current task list>
//!    [Assistant]: all done
//! ```
//!
//! [`ForcedExclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedExclude

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::TodoAutoPruneConfig;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;
use std::collections::HashMap;
use std::sync::Arc;

/// Returns true if a tool name belongs to the todo tool group.
fn is_todo_tool(name: &str) -> bool {
    name.starts_with("todo_")
}

/// A matched ToolCall with its index and the tool_call_id used to find its ToolResult.
struct CallInfo {
    /// Index of the ToolCall in history.
    index: usize,
    /// The entry ID of the ToolCall.
    entry_id: crate::feat::session::chat_entry::ChatEntryId,
    /// The tool_call_id used to match ToolCall → ToolResult.
    tool_call_id: String,
}

/// Todo auto-prune worker.
///
/// Scans history for all `todo_`-prefixed tool calls as one unified group.
/// Marks every pair except the most recent one as
/// [`ContextOverride::ForcedExclude`]. Pruning is immediate — no delay
/// threshold.
#[derive(Clone)]
pub struct TodoAutoPruneWorker {
    /// Configuration for the todo auto-prune strategy.
    pub config: TodoAutoPruneConfig,
}

/// Collect all ToolCalls and ToolResults for any `todo_` tool from history.
///
/// Returns a list of call info (in history order) and a map from tool_call_id
/// to (result_index, result_entry_id). This single-pass approach avoids the
/// forward-scan loops used by other workers — ToolResults are collected into
/// a HashMap by their `id` field, which directly matches the ToolCall's `id`.
fn collect_all_todo_pairs(
    history: &[ChatEntry],
) -> (
    Vec<CallInfo>,
    HashMap<String, (usize, crate::feat::session::chat_entry::ChatEntryId)>,
) {
    // ToolCalls in history order — their position determines which are "oldest".
    let mut calls: Vec<CallInfo> = Vec::new();
    // ToolResults keyed by tool_call_id — one result per call.
    let mut result_map: HashMap<String, (usize, crate::feat::session::chat_entry::ChatEntryId)> =
        HashMap::new();

    for (i, entry) in history.iter().enumerate() {
        match &entry.kind {
            ChatEntryKind::ToolCall { name, id, .. } if is_todo_tool(name) => {
                calls.push(CallInfo {
                    index: i,
                    entry_id: entry.id.clone(),
                    tool_call_id: id.clone(),
                });
            }
            ChatEntryKind::ToolResult { id, name, .. } if is_todo_tool(name) => {
                // Each ToolResult's `id` matches exactly one ToolCall's `id`.
                result_map.insert(id.clone(), (i, entry.id.clone()));
            }
            _ => {}
        }
    }

    (calls, result_map)
}

/// Build prune mutations for all-but-the-last call for a given tool name.
///
/// Calls are in history order (oldest first). All calls except the last
/// (most recent) are pruned. Only emits mutations for entries not already excluded.
fn build_prune_mutations(
    history: &[ChatEntry],
    calls: &[CallInfo],
    result_map: &HashMap<String, (usize, crate::feat::session::chat_entry::ChatEntryId)>,
) -> Vec<HistoryMutation> {
    // Need at least 2 calls to have something to prune.
    if calls.len() <= 1 {
        return Vec::new();
    }

    let mut mutations = Vec::new();

    // Prune all calls except the last one (most recent).
    for call_info in calls.iter().take(calls.len() - 1) {
        // Prune the ToolCall if not already excluded.
        if history[call_info.index].context_override != ContextOverride::ForcedExclude {
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: call_info.entry_id.clone(),
                value: ContextOverride::ForcedExclude,
            });
        }

        // Prune the corresponding ToolResult if it exists and isn't already excluded.
        if let Some((result_idx, result_entry_id)) = result_map.get(&call_info.tool_call_id)
            && history[*result_idx].context_override != ContextOverride::ForcedExclude
        {
            mutations.push(HistoryMutation::SetContextOverride {
                entry_id: result_entry_id.clone(),
                value: ContextOverride::ForcedExclude,
            });
        }
    }

    mutations
}

#[async_trait::async_trait]
impl HistoryWorker for TodoAutoPruneWorker {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "auto-prune-todo"
    }

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        let (calls, result_map) = collect_all_todo_pairs(&history);
        build_prune_mutations(&history, &calls, &result_map)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::preferences_actor::user_preferences::TodoAutoPruneConfig;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::protocol::SessionId;

    /// Helper: create a `todo_get_task_list` ToolCall + ToolResult pair.
    fn get_task_list_call_result(call_id: &str, content: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "todo_get_task_list", r#"{"phase_id":"p1"}"#),
            ChatEntry::tool_result(
                call_id,
                "todo_get_task_list",
                content,
                ToolResultStatus::Success,
            ),
        ]
    }

    /// Helper: create a `todo_complete_task` ToolCall + ToolResult pair.
    fn complete_task_call_result(call_id: &str, content: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "todo_complete_task", r#"{"task_id":"t1"}"#),
            ChatEntry::tool_result(
                call_id,
                "todo_complete_task",
                content,
                ToolResultStatus::Success,
            ),
        ]
    }

    /// Helper: create a `todo_add_phase` ToolCall + ToolResult pair.
    fn add_phase_call_result(call_id: &str, content: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "todo_add_phase", r#"{"description":"Build"}"#),
            ChatEntry::tool_result(
                call_id,
                "todo_add_phase",
                content,
                ToolResultStatus::Success,
            ),
        ]
    }

    /// Helper: create a `todo_add_task` ToolCall + ToolResult pair.
    fn add_task_call_result(call_id: &str, content: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(
                call_id,
                "todo_add_task",
                r#"{"phase_id":"p1","description":"Write code"}"#,
            ),
            ChatEntry::tool_result(call_id, "todo_add_task", content, ToolResultStatus::Success),
        ]
    }

    fn worker() -> TodoAutoPruneWorker {
        TodoAutoPruneWorker {
            config: TodoAutoPruneConfig { enabled: true },
        }
    }

    /// Evaluate the worker synchronously for tests.
    fn evaluate(history: Vec<ChatEntry>) -> Vec<HistoryMutation> {
        let w = worker();
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async { w.evaluate(&SessionId::new(), Arc::from(history)).await })
    }

    // --- Tests ---

    #[test]
    fn no_todo_calls_produces_no_mutations() {
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
    fn single_get_task_list_no_prune() {
        let call_result = get_task_list_call_result("tc-1", "task list here");
        let history = vec![call_result[0].clone(), call_result[1].clone()];
        let mutations = evaluate(history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn single_complete_task_no_prune() {
        let call_result = complete_task_call_result("tc-1", "task completed");
        let history = vec![call_result[0].clone(), call_result[1].clone()];
        let mutations = evaluate(history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn multiple_get_task_list_prunes_older() {
        let mut history = Vec::new();
        // First call (older — should be pruned).
        let cr1 = get_task_list_call_result("tc-1", "list v1");
        history.push(cr1[0].clone());
        history.push(cr1[1].clone());
        // Second call (most recent — should be kept).
        let cr2 = get_task_list_call_result("tc-2", "list v2");
        history.push(cr2[0].clone());
        history.push(cr2[1].clone());

        let mutations = evaluate(history);
        // Should prune: tc-1 ToolCall + tc-1 ToolResult = 2 mutations.
        assert_eq!(mutations.len(), 2);

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
            mutation_ids.contains(&cr1[0].id),
            "tc-1 ToolCall should be pruned"
        );
        assert!(
            mutation_ids.contains(&cr1[1].id),
            "tc-1 ToolResult should be pruned"
        );
    }

    #[test]
    fn multiple_complete_task_prunes_older() {
        let mut history = Vec::new();
        let cr1 = complete_task_call_result("tc-1", "completed t1");
        history.push(cr1[0].clone());
        history.push(cr1[1].clone());
        let cr2 = complete_task_call_result("tc-2", "completed t2");
        history.push(cr2[0].clone());
        history.push(cr2[1].clone());

        let mutations = evaluate(history);
        assert_eq!(mutations.len(), 2);

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
            mutation_ids.contains(&cr1[0].id),
            "tc-1 ToolCall should be pruned"
        );
        assert!(
            mutation_ids.contains(&cr1[1].id),
            "tc-1 ToolResult should be pruned"
        );
    }

    #[test]
    fn interleaved_todo_tools_pruned_as_group() {
        let mut history = Vec::new();
        // get_task_list v1 (older — should be pruned)
        let g1 = get_task_list_call_result("g-1", "list v1");
        history.push(g1[0].clone());
        history.push(g1[1].clone());
        // complete_task v1 (older — should be pruned)
        let c1 = complete_task_call_result("c-1", "completed t1");
        history.push(c1[0].clone());
        history.push(c1[1].clone());
        // get_task_list v2 (older — should be pruned)
        let g2 = get_task_list_call_result("g-2", "list v2");
        history.push(g2[0].clone());
        history.push(g2[1].clone());
        // complete_task v2 (most recent — kept)
        let c2 = complete_task_call_result("c-2", "completed t2");
        history.push(c2[0].clone());
        history.push(c2[1].clone());

        let mutations = evaluate(history);
        // Unified pruning: only c-2 survives. g-1, c-1, g-2 are pruned.
        // 2 mutations each for g-1, c-1, g-2 = 6.
        assert_eq!(mutations.len(), 6);

        let mutation_ids: Vec<_> = mutations
            .iter()
            .filter_map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, .. } => Some(entry_id.clone()),
                _ => None,
            })
            .collect();

        // g-1 and c-1 and g-2 should be pruned.
        assert!(mutation_ids.contains(&g1[0].id));
        assert!(mutation_ids.contains(&g1[1].id));
        assert!(mutation_ids.contains(&c1[0].id));
        assert!(mutation_ids.contains(&c1[1].id));
        assert!(mutation_ids.contains(&g2[0].id));
        assert!(mutation_ids.contains(&g2[1].id));
        // c-2 (most recent) should NOT be pruned.
        assert!(!mutation_ids.contains(&c2[0].id));
        assert!(!mutation_ids.contains(&c2[1].id));
    }

    #[test]
    fn already_excluded_no_duplicate_mutation() {
        let mut history = Vec::new();
        let cr1 = get_task_list_call_result("tc-1", "list v1");
        // Mark both as already excluded.
        let mut call = cr1[0].clone();
        call.context_override = ContextOverride::ForcedExclude;
        let mut result = cr1[1].clone();
        result.context_override = ContextOverride::ForcedExclude;
        history.push(call);
        history.push(result);

        let cr2 = get_task_list_call_result("tc-2", "list v2");
        history.push(cr2[0].clone());
        history.push(cr2[1].clone());

        let mutations = evaluate(history);
        // Both tc-1 entries are already excluded → 0 mutations.
        assert!(
            mutations.is_empty(),
            "should not produce mutations for already-excluded entries"
        );
    }

    #[test]
    fn tool_call_without_result_still_prunes_call() {
        let mut history = Vec::new();
        // Orphan ToolCall with no corresponding ToolResult.
        history.push(ChatEntry::tool_call(
            "tc-orphan",
            "todo_get_task_list",
            "{}",
        ));
        let orphan_id = history[0].id.clone();
        // Most recent call with result.
        let cr2 = get_task_list_call_result("tc-2", "list v2");
        history.push(cr2[0].clone());
        history.push(cr2[1].clone());

        let mutations = evaluate(history);
        // 1 mutation: the orphan ToolCall (no result to prune).
        assert_eq!(mutations.len(), 1);
        match &mutations[0] {
            HistoryMutation::SetContextOverride { entry_id, value } => {
                assert_eq!(*entry_id, orphan_id);
                assert_eq!(*value, ContextOverride::ForcedExclude);
            }
            other => panic!("expected SetContextOverride, got {other:?}"),
        }
    }

    #[test]
    fn three_calls_prunes_first_two() {
        let mut history = Vec::new();
        let cr1 = get_task_list_call_result("tc-1", "v1");
        history.push(cr1[0].clone());
        history.push(cr1[1].clone());
        let cr2 = get_task_list_call_result("tc-2", "v2");
        history.push(cr2[0].clone());
        history.push(cr2[1].clone());
        let cr3 = get_task_list_call_result("tc-3", "v3");
        history.push(cr3[0].clone());
        history.push(cr3[1].clone());

        let mutations = evaluate(history);
        // tc-1 call + result + tc-2 call + result = 4 mutations.
        assert_eq!(mutations.len(), 4);

        let mutation_ids: Vec<_> = mutations
            .iter()
            .filter_map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, .. } => Some(entry_id.clone()),
                _ => None,
            })
            .collect();

        // tc-1 and tc-2 pruned.
        assert!(mutation_ids.contains(&cr1[0].id));
        assert!(mutation_ids.contains(&cr1[1].id));
        assert!(mutation_ids.contains(&cr2[0].id));
        assert!(mutation_ids.contains(&cr2[1].id));
        // tc-3 (most recent) NOT pruned.
        assert!(!mutation_ids.contains(&cr3[0].id));
        assert!(!mutation_ids.contains(&cr3[1].id));
    }

    #[test]
    fn other_tool_calls_not_affected() {
        let mut history = Vec::new();
        // A read tool call (should not be touched).
        history.push(ChatEntry::tool_call(
            "rc-1",
            "read",
            r#"{"path": "/foo.rs"}"#,
        ));
        history.push(ChatEntry::tool_result(
            "rc-1",
            "read",
            "contents",
            ToolResultStatus::Success,
        ));
        // A todo_get_task_list call.
        let cr = get_task_list_call_result("tc-1", "list");
        history.push(cr[0].clone());
        history.push(cr[1].clone());

        let mutations = evaluate(history);
        // Only one todo_get_task_list call → nothing to prune.
        assert!(mutations.is_empty());
    }

    #[test]
    fn different_todo_tools_prunes_older() {
        let mut history = Vec::new();
        // add_task (older — should be pruned)
        let a1 = add_task_call_result("a-1", "created t1");
        history.push(a1[0].clone());
        history.push(a1[1].clone());
        // add_phase (newer — should be kept)
        let a2 = add_phase_call_result("a-2", "created phase");
        history.push(a2[0].clone());
        history.push(a2[1].clone());

        let mutations = evaluate(history);
        // a-1 call + result = 2 mutations.
        assert_eq!(mutations.len(), 2);

        let mutation_ids: Vec<_> = mutations
            .iter()
            .filter_map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, .. } => Some(entry_id.clone()),
                _ => None,
            })
            .collect();

        assert!(
            mutation_ids.contains(&a1[0].id),
            "a-1 ToolCall should be pruned"
        );
        assert!(
            mutation_ids.contains(&a1[1].id),
            "a-1 ToolResult should be pruned"
        );
        assert!(
            !mutation_ids.contains(&a2[0].id),
            "a-2 ToolCall should be kept"
        );
        assert!(
            !mutation_ids.contains(&a2[1].id),
            "a-2 ToolResult should be kept"
        );
    }

    #[test]
    fn mixed_todo_tools_keeps_only_last() {
        let mut history = Vec::new();
        // add_task (oldest)
        let a1 = add_task_call_result("a-1", "created t1");
        history.push(a1[0].clone());
        history.push(a1[1].clone());
        // add_phase (middle)
        let a2 = add_phase_call_result("a-2", "created phase");
        history.push(a2[0].clone());
        history.push(a2[1].clone());
        // get_task_list (newest — should be kept)
        let g1 = get_task_list_call_result("g-1", "list v1");
        history.push(g1[0].clone());
        history.push(g1[1].clone());

        let mutations = evaluate(history);
        // a-1 pair + a-2 pair = 4 mutations.
        assert_eq!(mutations.len(), 4);

        let mutation_ids: Vec<_> = mutations
            .iter()
            .filter_map(|m| match m {
                HistoryMutation::SetContextOverride { entry_id, .. } => Some(entry_id.clone()),
                _ => None,
            })
            .collect();

        // a-1 and a-2 pruned.
        assert!(mutation_ids.contains(&a1[0].id));
        assert!(mutation_ids.contains(&a1[1].id));
        assert!(mutation_ids.contains(&a2[0].id));
        assert!(mutation_ids.contains(&a2[1].id));
        // g-1 (most recent) NOT pruned.
        assert!(!mutation_ids.contains(&g1[0].id));
        assert!(!mutation_ids.contains(&g1[1].id));
    }
}
