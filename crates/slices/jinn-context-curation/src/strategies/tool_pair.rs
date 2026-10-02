//! Structural helpers shared by the tool-call/tool-result pairing strategies.
//!
//! Auto-prune strategies that reason about a *file* do so by pairing each
//! `ToolCall` with the `ToolResult` that answers it, then grouping those pairs
//! by the path the call touched. The pairing and path-extraction steps are the
//! same arithmetic in every one of those strategies; only the prune policy that
//! runs afterwards differs. They live here so that a change to how a pair is
//! resolved lands once.
//!
//! Override state is deliberately **not** consulted here. These helpers are
//! purely structural lookups; the suppression of protected entries happens at
//! mutation-emission time in each strategy's own `build_prune_mutations`.

use jinn_core_types::{ChatEntry, ChatEntryId, ChatEntryKind};

/// Tool names that modify files.
pub(super) const MODIFY_TOOLS: &[&str] = &["edit", "write"];

/// Returns `true` if the tool name is a file-modifying tool (`edit` or `write`).
pub(super) fn is_modify_tool(name: &str) -> bool {
    MODIFY_TOOLS.contains(&name)
}

/// Extract the `path` field from a tool call's JSON arguments string.
///
/// Returns `None` if the arguments cannot be parsed or the `path` field is
/// missing or not a string.
pub(super) fn extract_path_from_arguments(arguments: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    value
        .get("path")?
        .as_str()
        .map(std::borrow::ToOwned::to_owned)
}

/// Walk forward from a `ToolCall` at `call_idx` to find its matching `ToolResult`.
///
/// Returns `Some((result_entry_id, result_index))` if a match is found.
/// Returns `None` if no result exists (pending/orphaned).
pub(super) fn find_matching_result(
    history: &[ChatEntry],
    call_idx: usize,
    tool_call_id: &str,
) -> Option<(ChatEntryId, usize)> {
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
