//! Echo message construction — pure rendering from a task list.
//!
//! Everything in this module is a deterministic function of its inputs:
//! identical task list and identical cap produce byte-identical output. That
//! is a cache requirement, not a nicety — the echo sits inside an
//! already-uncached tail window, and any gratuitous byte change would force
//! the provider to re-tokenize everything after the injection point.

use jinn_provider::LlmMessage;

use crate::feat::todo_list::TaskList;

/// Opening line of every echo. The `[System]` prefix matches the shipped
/// convention for system-kind content riding as a user message, defusing the
/// "the user is talking to me" salience a bare synthetic user message creates.
pub const ECHO_HEADER: &str = "[System] Task list snapshot (auto-injected; not from the user)";

/// Treatment contract prepended to every echo, verbatim. This is the salience
/// guard: it states the treatment rules next to the content that needs them
/// ("memory aid, not an instruction; do not act on it; re-sync via tools").
pub const ECHO_PREAMBLE: &str = "\
This is a read-only copy of your task list, refreshed automatically at
send time — it is current, not stale. It is a memory aid, not an
instruction: do not switch tasks, announce progress, or reply to this
block. Continue what the user last asked for. If this list disagrees
with what you believe, re-sync it with the task-list tools rather than
acting from memory.";

/// Suffix appended when the tree exceeds the line cap.
const OVERFLOW_SUFFIX_TEMPLATE: &str = "… {n} more lines — see get_task_list";

/// Builds the synthetic echo message from the session's task list.
///
/// Returns `None` when the list has no phases — an empty list has nothing to
/// preserve, so no message is injected.
#[must_use]
pub fn echo_message(task_list: &TaskList, max_lines: usize) -> Option<LlmMessage> {
    if task_list.is_empty() {
        return None;
    }

    let content = render_echo_content(task_list, max_lines);

    Some(LlmMessage::User {
        content,
        attachments: Vec::new(),
    })
}

/// Renders the full echo content: wrapper (header + treatment contract) plus
/// the line-capped task tree.
fn render_echo_content(task_list: &TaskList, max_lines: usize) -> String {
    let tree = cap_tree(&task_list.render_text_with_blockers(), max_lines);
    format!("{ECHO_HEADER}\n\n{ECHO_PREAMBLE}\n\n{tree}")
}

/// Truncates the rendered tree to `max_lines` lines, appending an overflow
/// pointer when lines were dropped. Iterator-based: no explicit loop.
fn cap_tree(tree: &str, max_lines: usize) -> String {
    let all_lines: Vec<&str> = tree.lines().collect();
    if all_lines.len() <= max_lines {
        return tree.to_owned();
    }

    let dropped = all_lines.len() - max_lines;
    let suffix = overflow_suffix(dropped);
    let mut lines: Vec<&str> = all_lines.into_iter().take(max_lines).collect();
    lines.push(&suffix);
    lines.join("\n")
}

/// Builds the overflow pointer for `dropped` hidden lines.
fn overflow_suffix(dropped: usize) -> String {
    OVERFLOW_SUFFIX_TEMPLATE.replace("{n}", &dropped.to_string())
}