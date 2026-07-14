//! Conversion from chat entries to LLM messages.

use crate::feat::provider::llm_message::LlmMessage;
use crate::feat::tools_actor::tool_types::ToolCall;
use crate::protocol::{ChatEntry, ChatEntryKind, ContextOverride};

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
/// | Error | `LlmMessage::User` with `[Error]` prefix or actionable framing | `[Error]` prefix when `Default` (incl. pinned); actionable framing (`The user has shared...`) when `ForcedInclude` |
/// | Thinking | `LlmMessage::User` with `[Thinking]` prefix | Only when in context |
/// | Transient | `LlmMessage::User` with `[Transient]` prefix | Only when in context |
/// | Skill | `LlmMessage::System` with skill XML | Always in context by default |
/// | Compaction | `LlmMessage::User` with summary | Always in context by default |
/// Attaches a tool call to the most recent assistant message,
/// or creates an empty assistant message if none exists (orphan tool call).
fn push_tool_call_message(messages: &mut Vec<LlmMessage>, tool_call: ToolCall) {
    match messages.last_mut() {
        Some(LlmMessage::Assistant { tool_calls, .. }) => {
            tool_calls.get_or_insert_with(Vec::new).push(tool_call);
        }
        _ => {
            // Orphaned tool call - create an empty assistant message.
            messages.push(LlmMessage::Assistant {
                content: String::new(),
                tool_calls: Some(vec![tool_call]),
            });
        }
    }
}

/// Formats an error entry as a user message, using actionable framing for
/// `ForcedInclude` entries and the legacy `[Error]` prefix otherwise.
fn error_to_user_message(text: &str, context_override: ContextOverride) -> LlmMessage {
    let content = match context_override {
        ContextOverride::ForcedInclude => {
            format!("The user has shared the following output for you to address:\n\n{text}")
        }
        // Default and ForcedExclude (unreachable here; already
        // filtered by is_in_context). Pin alone does not trigger
        // the actionable framing.
        ContextOverride::Default | ContextOverride::ForcedExclude => {
            format!("[Error] {text}")
        }
    };
    LlmMessage::User {
        content,
        attachments: Vec::new(),
    }
}

/// Formats a compaction summary as a user message wrapping the summary in XML tags.
fn compaction_to_user_message(summary: &str) -> LlmMessage {
    LlmMessage::User {
        content: format!(
            "The conversation history before this point was compacted into the following summary:\n\n<summary>\n{summary}\n</summary>"
        ),
        attachments: Vec::new(),
    }
}
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
                    attachments: Vec::new(),
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
                let tool_call = ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                };
                push_tool_call_message(&mut messages, tool_call);
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
                    attachments: Vec::new(),
                });
            }
            ChatEntryKind::Error(text) => {
                messages.push(error_to_user_message(text, entry.context_override()));
            }
            // Thinking entries produce a User message with [Thinking] prefix
            // when in context (pinned or forced-include).
            ChatEntryKind::Thinking(text) => {
                messages.push(LlmMessage::User {
                    content: format!("[Thinking] {text}"),
                    attachments: Vec::new(),
                });
            }
            // Transient entries produce a User message with [Transient] prefix
            // when in context (pinned or forced-include).
            ChatEntryKind::Transient(text) => {
                messages.push(LlmMessage::User {
                    content: format!("[Transient] {text}"),
                    attachments: Vec::new(),
                });
            }
            ChatEntryKind::Compaction { summary, .. } => {
                messages.push(compaction_to_user_message(summary));
            }
            // Annotations are display-only and never enter LLM context.
            ChatEntryKind::Annotation { .. } => {}
        }
    }

    messages
}
