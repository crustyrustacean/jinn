//! Conversion from chat entries to LLM messages.

use crate::feat::provider::llm_message::LlmMessage;
use crate::feat::tools_actor::tool_types::ToolCall;
use crate::protocol::ChatEntry;
use crate::protocol::ChatEntryKind;

/// Convert chat history entries to LLM messages.
///
/// Produces messages for entries that are in context (as determined by
/// [`ChatEntry::is_in_context()`]). The `is_in_context()` guard at the
/// top of the loop ensures only eligible entries produce messages.
///
/// ## Message mapping
///
/// | Entry kind | LLM message | Notes |
/// |---|---|---|
/// | User | `LlmMessage::User` | Uses `expanded` content |
/// | Assistant | `LlmMessage::Assistant` | Tool calls attached from subsequent `ToolCall` entries |
/// | ToolCall | Attached to previous `Assistant` message | Or creates empty assistant |
/// | ToolResult | `LlmMessage::Tool` | |
/// | System | `LlmMessage::System` | Only when in context (pinned or forced-include) |
/// | Actor | `LlmMessage::User` with `[Actor: source]` prefix | Only when in context |
/// | Error | `LlmMessage::User` with `[Error]` prefix | Always in context by default |
/// | Thinking | `LlmMessage::User` with `[Thinking]` prefix | Only when in context |
/// | Transient | `LlmMessage::User` with `[Transient]` prefix | Only when in context |
/// | Skill | `LlmMessage::System` with skill XML | Always in context by default |
/// | Compaction | `LlmMessage::User` with summary | Always in context by default |
pub fn entries_to_messages(entries: &[ChatEntry]) -> Vec<LlmMessage> {
    let mut messages = Vec::new();

    for entry in entries {
        // Defensive: skip entries not in context.
        // The assembly handler pre-filters, but this prevents bugs
        // if entries_to_messages is called from other contexts.
        if !entry.is_in_context() {
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
            // System entries produce a System message when in context
            // (pinned or forced-include).
            ChatEntryKind::System(content) => {
                messages.push(LlmMessage::System {
                    content: content.clone(),
                });
            }
            // Actor entries produce a User message when in context
            // (pinned or forced-include).
            ChatEntryKind::Actor { source, text } => {
                messages.push(LlmMessage::User {
                    content: format!("[Actor: {source}] {text}"),
                });
            }
            // Error entries produce a User message with [Error] prefix.
            // Error is included in context by default.
            ChatEntryKind::Error(text) => {
                messages.push(LlmMessage::User {
                    content: format!("[Error] {text}"),
                });
            }
            // Thinking entries produce a User message with [Thinking] prefix
            // when in context (pinned or forced-include).
            ChatEntryKind::Thinking(text) => {
                messages.push(LlmMessage::User {
                    content: format!("[Thinking] {text}"),
                });
            }
            // Transient entries produce a User message with [Transient] prefix
            // when in context (pinned or forced-include).
            ChatEntryKind::Transient(text) => {
                messages.push(LlmMessage::User {
                    content: format!("[Transient] {text}"),
                });
            }
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
