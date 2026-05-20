//! Conversion from chat entries to LLM messages.

use crate::feat::provider::llm_message::LlmMessage;
use crate::feat::tools_actor::tool_types::ToolCall;
use crate::protocol::ChatEntry;
use crate::protocol::ChatEntryKind;

/// Convert chat history entries to LLM messages.
///
/// Includes `User`, `Assistant`, `ToolCall`, and `ToolResult` entries.
///
/// System and Actor entries are **skipped unless pinned**. When pinned:
/// - Pinned `System` entries produce [`LlmMessage::System`] messages.
/// - Pinned `Actor` entries produce [`LlmMessage::User`] messages with a
///   `[Actor: source]` prefix to identify the origin.
///
/// Unpinned System and Actor entries are excluded from the LLM conversation
/// context since they represent internal application state.
///
/// Assistant entries that follow a `ToolCall` + `ToolResult` sequence are
/// produced with their `tool_calls` field populated.
pub fn entries_to_messages(entries: &[ChatEntry]) -> Vec<LlmMessage> {
    let mut messages = Vec::new();

    for entry in entries {
        // Defensive: skip ignored entries that are not pinned.
        // The assembly handler pre-filters, but this prevents bugs
        // if entries_to_messages is called from other contexts.
        if entry.ignored && !entry.is_pinned() {
            continue;
        }
        match &entry.kind {
            ChatEntryKind::User { expanded, .. } => {
                messages.push(LlmMessage::User {
                    content: expanded.clone(),
                });
            }
            ChatEntryKind::Assistant(text) => {
                messages.push(LlmMessage::Assistant {
                    content: text.clone(),
                    tool_calls: None,
                });
            }
            ChatEntryKind::ToolCall {
                id,
                name,
                arguments,
            } => {
                // Attach tool calls to the most recent assistant message.
                // If there's no assistant message yet, create an empty one.
                let tool_call = ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                };
                match messages.last_mut() {
                    Some(LlmMessage::Assistant { tool_calls, .. }) => {
                        tool_calls.get_or_insert_with(Vec::new).push(tool_call);
                    }
                    _ => {
                        // Orphaned tool call — create an empty assistant message.
                        messages.push(LlmMessage::Assistant {
                            content: String::new(),
                            tool_calls: Some(vec![tool_call]),
                        });
                    }
                }
            }
            ChatEntryKind::ToolResult {
                id, name, content, ..
            } => {
                messages.push(LlmMessage::Tool {
                    tool_call_id: id.clone(),
                    name: name.clone(),
                    content: content.clone(),
                });
            }
            // System entries are only sent to the LLM when pinned.
            ChatEntryKind::System(content) => {
                if entry.is_pinned() {
                    messages.push(LlmMessage::System {
                        content: content.clone(),
                    });
                }
            }
            // Actor entries are only sent to the LLM when pinned.
            ChatEntryKind::Actor { source, text } => {
                if entry.is_pinned() {
                    messages.push(LlmMessage::User {
                        content: format!("[Actor: {source}] {text}"),
                    });
                }
            }
            // Error entries are ephemeral display / local status — not sent to the LLM.
            // Thinking entries are display-only — excluded from context assembly.
            // Transient entries are UI-only — excluded from prompt assembly.
            ChatEntryKind::Error(_)
            | ChatEntryKind::Thinking(_)
            | ChatEntryKind::Transient(_) => {}
            // Compaction entries produce a User message wrapping the summary.
            // The summary replaces all ignored entries before this point.
            ChatEntryKind::Compaction { summary, .. } => {
                messages.push(LlmMessage::User {
                    content: format!(
                        "The conversation history before this point was compacted into the following summary:\n\n<summary>\n{summary}\n</summary>"
                    ),
                });
            }
            // Skill entries produce System messages with the skill XML format.
            // Skills are always pinned, so they always produce a message.
            ChatEntryKind::Skill {
                name,
                location,
                content,
            } => {
                messages.push(LlmMessage::System {
                    content: format!(
                        "<skill name=\"{name}\" location=\"{location}\">\n{content}\n</skill>"
                    ),
                });
            }
        }
    }

    messages
}
