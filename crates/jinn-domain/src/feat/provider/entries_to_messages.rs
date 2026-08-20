//! Conversion from chat entries to LLM messages.

use crate::feat::provider::llm_message::LlmMessage;
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::feat::tools_actor::tool_types::ToolCall;
use crate::protocol::{ChatEntry, ChatEntryKind, ContextOverride};

/// Select context entries while retaining tool fragments for the validator and
/// an actual empty assistant that is the parent of a complete visible batch.
fn context_entries(entries: &[ChatEntry]) -> Vec<&ChatEntry> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            (entry.is_in_context()
                || matches!(
                    entry.kind,
                    ChatEntryKind::ToolCall { .. } | ChatEntryKind::ToolResult { .. }
                )
                || (has_tool_call_after(entries, index)
                    && matches!(entry.kind, ChatEntryKind::Assistant(_))))
            .then_some(entry)
        })
        .collect()
}

fn has_tool_call_after(entries: &[ChatEntry], index: usize) -> bool {
    entries
        .get(index + 1)
        .is_some_and(|entry| matches!(entry.kind, ChatEntryKind::ToolCall { .. }))
}

/// Formats an error entry as a user message, using actionable framing for
/// `ForcedInclude` entries and the legacy `[Error]` prefix otherwise.
fn error_to_user_message(text: &str, context_override: ContextOverride) -> LlmMessage {
    let content = match context_override {
        ContextOverride::ForcedInclude => {
            format!("The user has shared the following output for you to address:\n\n{text}")
        }
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

fn tool_batch_is_valid(
    calls: &[ToolCall],
    results: &[(&str, &str, &str, ToolResultStatus)],
) -> bool {
    let unique_call_ids = calls
        .iter()
        .all(|call| calls.iter().filter(|other| other.id == call.id).count() == 1);
    unique_call_ids
        && calls.len() == results.len()
        && calls
            .iter()
            .zip(results)
            .all(|(call, (id, name, _, status))| {
                call.id == *id && call.name == *name && *status != ToolResultStatus::Pending
            })
}

fn warn_malformed_batch(
    calls: &[ToolCall],
    results: &[(&str, &str, &str, ToolResultStatus)],
    reason: &str,
) {
    let call_details: Vec<(&str, &str)> = calls
        .iter()
        .map(|call| (call.id.as_str(), call.name.as_str()))
        .collect();
    let result_details: Vec<(&str, &str)> = results
        .iter()
        .map(|(id, name, _, _)| (*id, *name))
        .collect();
    tracing::warn!(
        call_details = ?call_details,
        result_details = ?result_details,
        reason,
        "dropping malformed tool-message batch from LLM context"
    );
}

/// Convert chat history entries to provider-neutral messages.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "conversion remains one ordered state machine"
)]
pub fn entries_to_messages(entries: &[ChatEntry]) -> Vec<LlmMessage> {
    let entries = context_entries(entries);
    let mut messages = Vec::new();
    let mut index = 0;

    while let Some(entry) = entries.get(index) {
        match &entry.kind {
            ChatEntryKind::Assistant(text) => {
                let mut call_index = index + 1;
                let mut calls = Vec::new();
                while let Some(next) = entries.get(call_index)
                    && let ChatEntryKind::ToolCall {
                        id,
                        name,
                        arguments,
                    } = &next.kind
                {
                    calls.push(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    });
                    call_index += 1;
                }

                if calls.is_empty() {
                    if entry.is_in_context() {
                        messages.push(LlmMessage::Assistant {
                            content: text.clone(),
                            tool_calls: None,
                        });
                    }
                    index += 1;
                    continue;
                }

                let mut result_index = call_index;
                let mut results = Vec::new();
                while let Some(next) = entries.get(result_index)
                    && let ChatEntryKind::ToolResult {
                        id,
                        name,
                        content,
                        status,
                        ..
                    } = &next.kind
                {
                    results.push((id.as_str(), name.as_str(), content.as_str(), *status));
                    result_index += 1;
                }

                let result_keys: Vec<(&str, &str, &str, ToolResultStatus)> = results.clone();
                let complete = tool_batch_is_valid(&calls, &result_keys);
                let group_forced_include =
                    entries.get(index..result_index).is_some_and(|members| {
                        members
                            .iter()
                            .any(|entry| entry.context_override() == ContextOverride::ForcedInclude)
                    });
                let parent_in_context = group_forced_include
                    || entry.is_in_context()
                    || (entry.is_empty_assistant()
                        && entry.context_override() != ContextOverride::ForcedExclude);
                let members_in_context = entries.get(index..result_index).is_some_and(|members| {
                    group_forced_include
                        || members.iter().all(|entry| {
                            entry.is_in_context()
                                || (entry.is_empty_assistant()
                                    && entry.context_override() != ContextOverride::ForcedExclude)
                        })
                });

                if !complete {
                    warn_malformed_batch(
                        &calls,
                        &result_keys,
                        "missing, duplicate, or mismatched tool result",
                    );
                } else if parent_in_context && members_in_context {
                    messages.push(LlmMessage::Assistant {
                        content: text.clone(),
                        tool_calls: Some(calls),
                    });
                    messages.extend(results.into_iter().map(|(id, name, content, _status)| {
                        LlmMessage::Tool {
                            tool_call_id: id.to_owned(),
                            name: name.to_owned(),
                            content: content.to_owned(),
                        }
                    }));
                } else {
                    // A complete batch with an excluded member is omitted atomically.
                }
                index = result_index.max(call_index);
            }
            ChatEntryKind::ToolCall { id, name, .. } => {
                tracing::warn!(
                    call_id = %id,
                    tool = %name,
                    "dropping orphan tool call from LLM context"
                );
                index += 1;
            }
            ChatEntryKind::ToolResult { id, name, .. } => {
                tracing::warn!(
                    call_id = %id,
                    tool = %name,
                    "dropping orphan tool result from LLM context"
                );
                index += 1;
            }
            ChatEntryKind::User {
                expanded,
                attachments,
                ..
            } => {
                if entry.is_in_context() {
                    messages.push(LlmMessage::User {
                        content: expanded.clone(),
                        attachments: attachments.clone(),
                    });
                }
                index += 1;
            }
            ChatEntryKind::System(content) => {
                if entry.is_in_context() {
                    messages.push(LlmMessage::System {
                        content: content.clone(),
                    });
                }
                index += 1;
            }
            ChatEntryKind::Actor { source, text } => {
                if entry.is_in_context() {
                    messages.push(LlmMessage::User {
                        content: format!("[Actor: {source}] {text}"),
                        attachments: Vec::new(),
                    });
                }
                index += 1;
            }
            ChatEntryKind::Error(text) => {
                if entry.is_in_context() {
                    messages.push(error_to_user_message(text, entry.context_override()));
                }
                index += 1;
            }
            ChatEntryKind::Thinking(text) => {
                if entry.is_in_context() {
                    messages.push(LlmMessage::User {
                        content: format!("[Thinking] {text}"),
                        attachments: Vec::new(),
                    });
                }
                index += 1;
            }
            ChatEntryKind::Transient(text) => {
                if entry.is_in_context() {
                    messages.push(LlmMessage::User {
                        content: format!("[Transient] {text}"),
                        attachments: Vec::new(),
                    });
                }
                index += 1;
            }
            ChatEntryKind::Compaction { summary, .. } => {
                if entry.is_in_context() {
                    messages.push(compaction_to_user_message(summary));
                }
                index += 1;
            }
            ChatEntryKind::Annotation { .. } => {
                index += 1;
            }
        }
    }

    messages
}
