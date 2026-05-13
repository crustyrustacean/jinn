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
        match &entry.kind {
            ChatEntryKind::User(text) => {
                messages.push(LlmMessage::User {
                    content: text.clone(),
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
            // Table and Error entries are ephemeral display / local status — not sent to the LLM.
            ChatEntryKind::Table(_) | ChatEntryKind::Error(_) => {}
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PinPosition;

    #[rstest::rstest]
    fn entries_to_messages_converts_user_entries() {
        // Given a user chat entry.
        let entries = vec![ChatEntry::user("hello")];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then a single user message with correct content is produced.
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0],
            LlmMessage::User {
                content: "hello".into()
            }
        );
    }

    #[rstest::rstest]
    fn entries_to_messages_converts_assistant_entries() {
        // Given an assistant chat entry.
        let entries = vec![ChatEntry::assistant("hi there")];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then a single assistant message with correct content is produced.
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0],
            LlmMessage::Assistant {
                content: "hi there".into(),
                tool_calls: None,
            }
        );
    }

    #[rstest::rstest]
    fn entries_to_messages_skips_system_and_actor() {
        // Given entries of all kinds.
        let entries = vec![
            ChatEntry::system("ready"),
            ChatEntry::user("hello"),
            ChatEntry::actor("echo", "HELLO"),
            ChatEntry::assistant("hi"),
        ];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then only user and assistant messages are included.
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0],
            LlmMessage::User {
                content: "hello".into()
            }
        );
        assert_eq!(
            messages[1],
            LlmMessage::Assistant {
                content: "hi".into(),
                tool_calls: None,
            }
        );
    }

    #[rstest::rstest]
    fn entries_to_messages_empty_input() {
        // Given no entries.
        let entries: Vec<ChatEntry> = vec![];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then no messages are produced.
        assert!(messages.is_empty());
    }

    #[rstest::rstest]
    fn orphaned_tool_call_produces_assistant_message() {
        // Given a tool call entry (orphaned — no preceding assistant).
        let entries = vec![ChatEntry::tool_call("call_1", "echo", r#"{"input":"hi"}"#)];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then an empty assistant message with tool_calls is produced.
        assert_eq!(messages.len(), 1);
        assert!(matches!(&messages[0], LlmMessage::Assistant { .. }));
    }

    #[rstest::rstest]
    fn entries_to_messages_attaches_tool_calls_to_assistant() {
        // Given an assistant entry followed by a tool call entry.
        let entries = vec![
            ChatEntry::assistant("let me check"),
            ChatEntry::tool_call("call_1", "echo", r#"{"input":"hi"}"#),
        ];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then one assistant message with tool_calls is produced.
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            LlmMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "let me check");
                let calls = tool_calls.as_ref().expect("should have tool_calls");
                assert_eq!(calls.len(), 1);
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn entries_to_messages_converts_tool_result_entries() {
        // Given a tool result entry.
        let entries = vec![ChatEntry::tool_result("call_1", "echo", "hi", true)];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then a Tool message is produced.
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0],
            LlmMessage::Tool {
                tool_call_id: "call_1".into(),
                name: "echo".into(),
                content: "hi".into(),
            }
        );
    }

    #[rstest::rstest]
    fn tool_loop_produces_four_messages() {
        // Given a full tool loop: user → assistant → tool call → tool result → assistant.
        let entries = vec![
            ChatEntry::user("what time is it?"),
            ChatEntry::assistant(""),
            ChatEntry::tool_call("call_1", "get_time", "{}"),
            ChatEntry::tool_result("call_1", "get_time", "12:00", true),
            ChatEntry::assistant("It's 12:00!"),
        ];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then four messages are produced.
        assert_eq!(messages.len(), 4);
    }

    #[rstest::rstest]
    fn multiple_tool_calls_produce_one_assistant_message() {
        // Given an assistant entry followed by multiple tool call entries.
        let entries = vec![
            ChatEntry::assistant("checking both"),
            ChatEntry::tool_call("call_1", "echo", r#"{"input":"a"}"#),
            ChatEntry::tool_call("call_2", "get_time", "{}"),
        ];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then one assistant message is produced.
        assert_eq!(messages.len(), 1);
        assert!(matches!(&messages[0], LlmMessage::Assistant { .. }));
    }

    #[rstest::rstest]
    fn system_entries_skipped_between_tools() {
        // Given entries with system and actor entries between tool entries.
        let entries = vec![
            ChatEntry::user("go"),
            ChatEntry::assistant(""),
            ChatEntry::tool_call("call_1", "echo", "{}"),
            ChatEntry::system("some status"),
            ChatEntry::actor("actor-x", "doing work"),
            ChatEntry::tool_result("call_1", "echo", "ok", true),
        ];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then system and actor entries are skipped, producing only 3 messages.
        assert_eq!(messages.len(), 3);
        assert!(matches!(&messages[0], LlmMessage::User { .. }));
        assert!(matches!(&messages[1], LlmMessage::Assistant { .. }));
        assert!(matches!(&messages[2], LlmMessage::Tool { .. }));
    }

    #[rstest::rstest]
    fn pinned_system_entry_produces_system_message() {
        // Given a pinned System entry.
        let entries = vec![ChatEntry::system("important instruction").with_pin(PinPosition::Top)];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then a System message is produced.
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0],
            LlmMessage::System {
                content: "important instruction".into(),
            }
        );
    }

    #[rstest::rstest]
    fn pinned_actor_entry_produces_user_message() {
        // Given a pinned Actor entry.
        let entries = vec![ChatEntry::actor("echo", "HELLO").with_pin(PinPosition::Relative)];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then a User message with the actor prefix is produced.
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0],
            LlmMessage::User {
                content: "[Actor: echo] HELLO".into(),
            }
        );
    }

    #[rstest::rstest]
    fn unpinned_system_entry_still_skipped() {
        // Given an unpinned System entry.
        let entries = vec![ChatEntry::system("ready")];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then no messages are produced.
        assert!(messages.is_empty());
    }

    #[rstest::rstest]
    fn unpinned_actor_entry_still_skipped() {
        // Given an unpinned Actor entry.
        let entries = vec![ChatEntry::actor("echo", "HELLO")];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then no messages are produced.
        assert!(messages.is_empty());
    }

    #[rstest::rstest]
    fn mixed_pinned_and_unpinned_entries() {
        // Given a mix of pinned and unpinned System/Actor entries.
        let entries = vec![
            ChatEntry::system("unpinned system"),
            ChatEntry::system("pinned system").with_pin(PinPosition::Top),
            ChatEntry::actor("a", "unpinned actor"),
            ChatEntry::actor("b", "pinned actor").with_pin(PinPosition::Bottom),
        ];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then only the pinned entries appear.
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0],
            LlmMessage::System {
                content: "pinned system".into(),
            }
        );
        assert_eq!(
            messages[1],
            LlmMessage::User {
                content: "[Actor: b] pinned actor".into(),
            }
        );
    }

    #[rstest::rstest]
    fn pinned_system_appears_first() {
        // Given a pinned System entry alongside User and Assistant entries.
        let entries = vec![
            ChatEntry::system("always include").with_pin(PinPosition::Top),
            ChatEntry::user("hello"),
            ChatEntry::assistant("hi"),
        ];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then the pinned System entry appears first.
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[0],
            LlmMessage::System {
                content: "always include".into(),
            }
        );
    }

    #[rstest::rstest]
    fn pinned_user_and_assistant_entries_unaffected() {
        // Given pinned User and Assistant entries.
        let entries = vec![
            ChatEntry::user("hello").with_pin(PinPosition::Relative),
            ChatEntry::assistant("hi").with_pin(PinPosition::Relative),
        ];

        // When converting to messages.
        let messages = entries_to_messages(&entries);

        // Then pinning does not change their conversion — they are included as normal.
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0],
            LlmMessage::User {
                content: "hello".into(),
            }
        );
        assert_eq!(
            messages[1],
            LlmMessage::Assistant {
                content: "hi".into(),
                tool_calls: None,
            }
        );
    }
}
