//! Serialize chat entries to labeled text for the compaction LLM prompt.
//!
//! Produces a human-readable representation of the conversation that the
//! summarization model can process. Tool results are truncated to keep
//! the serialization within reasonable bounds.

use crate::protocol::{ChatEntry, ChatEntryKind};

/// Maximum characters to include from a tool result in the serialization.
const TOOL_RESULT_MAX_CHARS: usize = 2000;

/// Serialize a slice of chat entries into labeled text.
///
/// Each entry produces one or more labeled lines:
/// - `[User]: <text>`
/// - `[Assistant]: <text>`
/// - `[Tool call]: name(arguments)`
/// - `[Tool result]: <content>` (truncated to ~2000 chars)
///
/// System, Error, Thinking, Info, Table, Skill, and Compaction entries
/// are skipped — they are not relevant to the summarization prompt.
pub fn serialize_entries_for_compaction(entries: &[ChatEntry]) -> String {
    let mut lines = Vec::new();

    for entry in entries {
        match &entry.kind {
            ChatEntryKind::User { display, .. } => {
                lines.push(format!("[User]: {display}"));
            }
            ChatEntryKind::Assistant(text) => {
                lines.push(format!("[Assistant]: {text}"));
            }
            ChatEntryKind::ToolCall {
                name, arguments, ..
            } => {
                lines.push(format!("[Tool call]: {name}({arguments})"));
            }
            ChatEntryKind::ToolResult { name, content, .. } => {
                let truncated = if content.len() > TOOL_RESULT_MAX_CHARS {
                    let mut end = TOOL_RESULT_MAX_CHARS;
                    // Try to break at a newline or space.
                    if let Some(pos) = content[..TOOL_RESULT_MAX_CHARS].rfind('\n') {
                        end = pos;
                    } else if let Some(pos) = content[..TOOL_RESULT_MAX_CHARS].rfind(' ') {
                        end = pos;
                    }
                    format!("{}... (truncated)", &content[..end])
                } else {
                    content.clone()
                };
                lines.push(format!("[Tool result] {name}: {truncated}"));
            }
            ChatEntryKind::Actor { source, text } => {
                lines.push(format!("[Actor: {source}]: {text}"));
            }
            // Skip entries that don't contribute to the conversation summary.
            ChatEntryKind::System(_)
            | ChatEntryKind::Error(_)
            | ChatEntryKind::Thinking(_)
            | ChatEntryKind::Info(_)
            | ChatEntryKind::Table(_)
            | ChatEntryKind::Skill { .. }
            | ChatEntryKind::Compaction { .. } => {}
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn serializes_user_entry() {
        let entries = vec![ChatEntry::user("Hello world")];
        let result = serialize_entries_for_compaction(&entries);
        assert_eq!(result, "[User]: Hello world");
    }

    #[test]
    fn serializes_assistant_entry() {
        let entries = vec![ChatEntry::assistant("Hi there")];
        let result = serialize_entries_for_compaction(&entries);
        assert_eq!(result, "[Assistant]: Hi there");
    }

    #[test]
    fn serializes_tool_call_entry() {
        let entries = vec![ChatEntry::tool_call("id1", "bash", r#"{"command":"ls"}"#)];
        let result = serialize_entries_for_compaction(&entries);
        assert_eq!(result, r#"[Tool call]: bash({"command":"ls"})"#);
    }

    #[test]
    fn serializes_tool_result_entry() {
        let entries = vec![ChatEntry::tool_result(
            "id1",
            "bash",
            "file1.txt\nfile2.txt",
            crate::feat::session::tool_result_status::ToolResultStatus::Success,
        )];
        let result = serialize_entries_for_compaction(&entries);
        assert_eq!(result, "[Tool result] bash: file1.txt\nfile2.txt");
    }

    #[test]
    fn truncates_long_tool_result() {
        let long_content = "x".repeat(5000);
        let entries = vec![ChatEntry::tool_result(
            "id1",
            "bash",
            &long_content,
            crate::feat::session::tool_result_status::ToolResultStatus::Success,
        )];
        let result = serialize_entries_for_compaction(&entries);
        assert!(result.contains("truncated"));
        assert!(result.len() < 3000);
    }

    #[test]
    fn skips_system_entry() {
        let entries = vec![ChatEntry::system("ready")];
        let result = serialize_entries_for_compaction(&entries);
        assert!(result.is_empty());
    }

    #[test]
    fn skips_thinking_entry() {
        let entries = vec![ChatEntry::thinking("reasoning")];
        let result = serialize_entries_for_compaction(&entries);
        assert!(result.is_empty());
    }

    #[test]
    fn serializes_mixed_entries() {
        let entries = vec![
            ChatEntry::user("fix the bug"),
            ChatEntry::assistant("let me check"),
            ChatEntry::tool_call("id1", "bash", r#"{"command":"ls"}"#),
            ChatEntry::tool_result(
                "id1",
                "bash",
                "file.rs",
                crate::feat::session::tool_result_status::ToolResultStatus::Success,
            ),
            ChatEntry::assistant("done"),
        ];
        let result = serialize_entries_for_compaction(&entries);
        let lines: Vec<&str> = result.split('\n').collect();
        assert_eq!(lines.len(), 5);
        assert!(lines[0].starts_with("[User]"));
        assert!(lines[1].starts_with("[Assistant]"));
        assert!(lines[2].starts_with("[Tool call]"));
        assert!(lines[3].starts_with("[Tool result]"));
        assert!(lines[4].starts_with("[Assistant]"));
    }
}
