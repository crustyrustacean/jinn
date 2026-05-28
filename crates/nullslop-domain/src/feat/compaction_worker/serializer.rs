//! Serialize chat entries to labeled text for the compaction LLM prompt.
//!
//! Produces a human-readable representation of the conversation that the
//! summarization model can process. Tool results are truncated to keep
//! the serialization within reasonable bounds.

use unicode_segmentation::UnicodeSegmentation;

use crate::protocol::{ChatEntry, ChatEntryKind};

/// Maximum bytes to include from a tool result in the serialization.
const TOOL_RESULT_MAX_BYTES: usize = 2000;

/// Find the largest byte offset within `max_bytes` that falls on a grapheme boundary.
///
/// Returns `text.len()` if the entire string fits within `max_bytes`.
fn grapheme_safe_end(text: &str, max_bytes: usize) -> usize {
    if text.len() <= max_bytes {
        return text.len();
    }
    let mut end = 0;
    for (byte_idx, grapheme) in text.grapheme_indices(true) {
        let next_end = byte_idx + grapheme.len();
        if next_end > max_bytes {
            break;
        }
        end = next_end;
    }
    end
}

/// Serialize a slice of chat entries into labeled text.
///
/// Each entry produces one or more labeled lines:
/// - `[User]: <text>`
/// - `[Assistant]: <text>`
/// - `[Tool call]: name(arguments)`
/// - `[Tool result]: <content>` (truncated to ~2000 bytes)
///
/// System, Error, Thinking, Transient, Table, Skill, and Compaction entries
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
                let truncated = if content.len() > TOOL_RESULT_MAX_BYTES {
                    let safe_end = grapheme_safe_end(content, TOOL_RESULT_MAX_BYTES);
                    let candidate = &content[..safe_end];
                    let mut end = safe_end;
                    // Try to break at a newline or space.
                    if let Some(pos) = candidate.rfind('\n') {
                        end = pos;
                    } else if let Some(pos) = candidate.rfind(' ') {
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
            | ChatEntryKind::Transient(_)
            | ChatEntryKind::Skill { .. }
            | ChatEntryKind::Compaction { .. } => {}
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
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

    #[test]
    fn truncates_tool_result_with_multibyte_at_boundary() {
        // Given content where byte 2000 falls in the middle of an em-dash (3 bytes).
        // 1999 ASCII 'x' chars + "—" (em-dash, 3 bytes) + more text.
        let mut content = "x".repeat(1999);
        content.push('—');
        content.push_str("more text");

        let entries = vec![ChatEntry::tool_result(
            "id1",
            "bash",
            &content,
            crate::feat::session::tool_result_status::ToolResultStatus::Success,
        )];

        // When serializing for compaction.
        let result = serialize_entries_for_compaction(&entries);

        // Then it does not panic and contains the truncation marker.
        assert!(result.contains("... (truncated)"));
    }

    #[test]
    fn truncates_tool_result_with_emoji() {
        // Given content with emoji exceeding 2000 bytes.
        let content = "🎉".repeat(1000); // Each 🎉 is 4 bytes, so 4000 bytes total.

        let entries = vec![ChatEntry::tool_result(
            "id1",
            "bash",
            &content,
            crate::feat::session::tool_result_status::ToolResultStatus::Success,
        )];

        // When serializing for compaction.
        let result = serialize_entries_for_compaction(&entries);

        // Then it does not panic and contains the truncation marker.
        assert!(result.contains("... (truncated)"));
    }

    // --- Mutant-killing tests for grapheme_safe_end ---

    #[test]
    fn grapheme_safe_end_returns_len_for_short_string() {
        // Given a string shorter than max_bytes.
        let text = "hello";

        // When computing safe end.
        let end = grapheme_safe_end(text, 100);

        // Then it returns the full string length.
        assert_eq!(end, 5);
    }

    #[test]
    fn grapheme_safe_end_returns_len_for_exact_match() {
        // Given a string exactly at max_bytes.
        let text = "hello";

        // When computing safe end.
        let end = grapheme_safe_end(text, 5);

        // Then it returns the full string length (text fits within max_bytes).
        assert_eq!(end, 5);
    }

    #[test]
    fn grapheme_safe_end_truncates_at_grapheme_boundary() {
        // Given a string with multibyte chars exceeding max.
        // "abcé" = a(1) + b(1) + c(1) + é(2) = 5 bytes.
        let text = "abcéxyz";

        // When max_bytes is 4 (falls in the middle of é).
        let end = grapheme_safe_end(text, 4);

        // Then it returns 3 ("abc" — the last full grapheme before byte 4).
        assert_eq!(end, 3);
    }

    #[test]
    fn grapheme_safe_end_stops_before_boundary() {
        // Given "abcdefgh" (8 bytes) with max 6.
        let text = "abcdefgh";

        // When computing safe end.
        let end = grapheme_safe_end(text, 6);

        // Then it returns 6 ("abcdef").
        assert_eq!(end, 6);
    }

    #[test]
    fn grapheme_safe_end_zero_max_bytes() {
        // Given a non-empty string with max 0.
        let text = "hello";

        // When computing safe end.
        let end = grapheme_safe_end(text, 0);

        // Then it returns 0 (no grapheme fits).
        assert_eq!(end, 0);
    }

    #[test]
    fn grapheme_safe_end_empty_string() {
        // Given an empty string.
        let text = "";

        // When computing safe end with any max.
        let end = grapheme_safe_end(text, 10);

        // Then it returns 0.
        assert_eq!(end, 0);
    }

    #[test]
    fn grapheme_safe_end_single_multibyte_grapheme() {
        // Given a single emoji (4 bytes) with max 3.
        let text = "🎉";

        // When computing safe end.
        let end = grapheme_safe_end(text, 3);

        // Then it returns 0 (the emoji doesn't fit).
        assert_eq!(end, 0);
    }

    #[test]
    fn grapheme_safe_end_single_multibyte_grapheme_fits() {
        // Given a single emoji (4 bytes) with max 4.
        let text = "🎉";

        // When computing safe end.
        let end = grapheme_safe_end(text, 4);

        // Then it returns 4 (the emoji fits exactly).
        assert_eq!(end, 4);
    }

    #[test]
    fn serialize_tool_result_at_exact_boundary() {
        // Given content exactly at TOOL_RESULT_MAX_BYTES (2000 bytes).
        let content = "x".repeat(2000);
        let entries = vec![ChatEntry::tool_result(
            "id1",
            "bash",
            &content,
            crate::feat::session::tool_result_status::ToolResultStatus::Success,
        )];

        // When serializing.
        let result = serialize_entries_for_compaction(&entries);

        // Then it is NOT truncated (content fits within limit).
        assert!(!result.contains("truncated"));
    }
}
