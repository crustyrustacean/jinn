#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use crate::feat::provider::entries_to_messages::entries_to_messages;
use crate::feat::session::chat_entry::ChangeSource;
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::{ChatEntry, LlmMessage, PinPosition};

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
    // Given entries of all kinds (unpinned System and Actor are not in context).
    let entries = vec![
        ChatEntry::system("ready"),
        ChatEntry::user("hello"),
        ChatEntry::actor("echo", "HELLO"),
        ChatEntry::assistant("hi"),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then only user and assistant messages are included.
    // Unpinned System and Actor are excluded from context by default.
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
    // Given a tool call entry (orphaned - no preceding assistant).
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
    let entries = vec![ChatEntry::tool_result(
        "call_1",
        "echo",
        "hi",
        ToolResultStatus::Success,
    )];

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
        ChatEntry::tool_result("call_1", "get_time", "12:00", ToolResultStatus::Success),
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
        ChatEntry::tool_result("call_1", "echo", "ok", ToolResultStatus::Success),
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

    // Then pinning does not change their conversion - they are included as normal.
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

#[rstest::rstest]
fn entries_to_messages_skips_thinking_entries() {
    // Given a history with thinking (unpinned, not in context), user, and assistant entries.
    let entries = vec![
        ChatEntry::thinking("reasoning here"),
        ChatEntry::user("hello"),
        ChatEntry::assistant("hi"),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then thinking is excluded (not in context by default), producing only user and assistant.
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
fn transient_entries_are_skipped() {
    // Given Transient (unpinned, not in context), user, and assistant entries.
    let entries = vec![
        ChatEntry::transient("welcome"),
        ChatEntry::user("hello"),
        ChatEntry::assistant("hi"),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then Transient is excluded (not in context by default), producing only user and assistant.
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
fn compaction_entry_produces_user_message_with_summary() {
    // Given a compaction entry.
    let entries = vec![ChatEntry {
        id: crate::protocol::ChatEntryId::new(),
        timestamp: jiff::Timestamp::now(),
        kind: crate::protocol::ChatEntryKind::Compaction {
            summary: "User asked to fix a bug. Work completed.".to_owned(),
            tokens_before: 5000,
            tokens_after: 250,
            entries_compacted: 10,
            model_used: "test/model".to_owned(),
        },
        pin_position: None,
        context_override: crate::protocol::ContextOverride::Default,
        context_history: Vec::new(),
    }];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then a User message with the wrapped summary is produced.
    assert_eq!(messages.len(), 1);
    let content = match &messages[0] {
        LlmMessage::User { content } => content.clone(),
        other => panic!("expected User, got {other:?}"),
    };
    assert!(content.contains("compacted into the following summary"));
    assert!(content.contains("<summary>"));
    assert!(content.contains("User asked to fix a bug. Work completed."));
    assert!(content.contains("</summary>"));
}

#[rstest::rstest]
fn ignored_user_entry_is_skipped() {
    // Given an ignored user entry.
    let entries = vec![ChatEntry::user("hello").with_ignored(true)];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then no messages are produced.
    assert!(messages.is_empty());
}

#[rstest::rstest]
fn ignored_assistant_entry_is_skipped() {
    // Given an ignored assistant entry.
    let entries = vec![ChatEntry::assistant("response").with_ignored(true)];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then no messages are produced.
    assert!(messages.is_empty());
}

#[rstest::rstest]
fn ignored_pinned_entry_is_included() {
    // Given an ignored but pinned user entry.
    let entries = vec![
        ChatEntry::user("important")
            .with_pin(PinPosition::Relative)
            .with_ignored(true),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then the entry is included (pin overrides ignore).
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        LlmMessage::User {
            content: "important".into()
        }
    );
}

#[rstest::rstest]
fn ignored_entry_mixed_with_active_entries() {
    // Given a mix of ignored and active entries.
    let entries = vec![
        ChatEntry::user("first").with_ignored(true),
        ChatEntry::user("second"),
        ChatEntry::assistant("response").with_ignored(true),
        ChatEntry::assistant("final"),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then only active entries are included.
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0],
        LlmMessage::User {
            content: "second".into()
        }
    );
    assert_eq!(
        messages[1],
        LlmMessage::Assistant {
            content: "final".into(),
            tool_calls: None,
        }
    );
}

#[rstest::rstest]
fn message_order_after_compaction() {
    // Given a history with: compacted entries, compaction summary, recent entries.
    let entries = vec![
        ChatEntry::user("old question").with_ignored(true),
        ChatEntry::assistant("old answer").with_ignored(true),
        ChatEntry {
            id: crate::protocol::ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: crate::protocol::ChatEntryKind::Compaction {
                summary: "The user asked about X and was told Y".to_owned(),
                tokens_before: 500,
                tokens_after: 25,
                entries_compacted: 2,
                model_used: "test/model".to_owned(),
            },
            pin_position: None,
            context_override: crate::protocol::ContextOverride::Default,
            context_history: Vec::new(),
        },
        ChatEntry::user("new question"),
        ChatEntry::assistant("new answer"),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then the order is: compaction summary → new question → new answer.
    assert_eq!(messages.len(), 3);
    // Compaction summary as User message.
    assert!(
        matches!(&messages[0], LlmMessage::User { content } if content.contains("The user asked about X"))
    );
    // Recent entries follow.
    assert_eq!(
        messages[1],
        LlmMessage::User {
            content: "new question".into()
        }
    );
    assert_eq!(
        messages[2],
        LlmMessage::Assistant {
            content: "new answer".into(),
            tool_calls: None,
        }
    );
}

// --- Error entry tests ---

#[rstest::rstest]
fn error_entry_default_is_skipped() {
    // Given an Error entry (default context override).
    let entries = vec![ChatEntry::error("something went wrong")];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then no messages are produced (Error is excluded from context by default).
    assert!(messages.is_empty());
}

#[rstest::rstest]
fn error_entry_between_user_and_assistant() {
    // Given Error, User, and Assistant entries.
    let entries = vec![
        ChatEntry::user("hello"),
        ChatEntry::error("connection lost"),
        ChatEntry::assistant("hi"),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then only User and Assistant produce messages (Error is excluded by default).
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
fn error_entry_forced_exclude_is_skipped() {
    // Given an Error entry with ForcedExclude.
    use crate::protocol::ContextOverride;
    let entries = vec![
        ChatEntry::error("ignored error").with_context_override(ContextOverride::ForcedExclude),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then no messages are produced.
    assert!(messages.is_empty());
}

#[rstest::rstest]
fn error_entry_forced_include_produces_user_message() {
    // Given an Error entry with ForcedInclude (not pinned).
    use crate::protocol::ContextOverride;
    let entries = vec![
        ChatEntry::error("important error").with_context_override(ContextOverride::ForcedInclude),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then a User message is produced with the actionable framing that
    // signals to the LLM the user wants it to address the contents
    // (not the legacy `[Error]` prefix, which primes investigation).
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        LlmMessage::User {
            content: "The user has shared the following output for you to address:\n\nimportant error".into()
        }
    );
}

#[rstest::rstest]
fn pinned_error_with_default_override_produces_error_prefix() {
    // Given a pinned Error entry with Default override (in context via
    // the pin, not via ForcedInclude). This is the only path that
    // reaches the Default branch of the override match in
    // entries_to_messages, since unpinned Default Errors are filtered
    // out by is_in_context upstream.
    let entries = vec![ChatEntry::error("pinned error").with_pin(PinPosition::Top)];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then the legacy [Error] prefix is used. Pinning alone does not
    // trigger the actionable framing - only ForcedInclude does.
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        LlmMessage::User {
            content: "[Error] pinned error".into()
        }
    );
}

#[rstest::rstest]
fn pinned_thinking_entry_produces_user_message() {
    // Given a pinned Thinking entry.
    let entries = vec![ChatEntry::thinking("reasoning").with_pin(PinPosition::Top)];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then a User message with [Thinking] prefix is produced.
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        LlmMessage::User {
            content: "[Thinking] reasoning".into()
        }
    );
}

#[rstest::rstest]
fn pinned_transient_entry_produces_user_message() {
    // Given a pinned Transient entry.
    let entries = vec![ChatEntry::transient("welcome").with_pin(PinPosition::Top)];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then a User message with [Transient] prefix is produced.
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        LlmMessage::User {
            content: "[Transient] welcome".into()
        }
    );
}

#[rstest::rstest]
fn forced_include_system_entry_produces_system_message() {
    // Given a System entry with ForcedInclude (not pinned).
    use crate::protocol::ContextOverride;
    let entries =
        vec![ChatEntry::system("important").with_context_override(ContextOverride::ForcedInclude)];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then a System message is produced (forced-include overrides kind default).
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        LlmMessage::System {
            content: "important".into(),
        }
    );
}

#[rstest::rstest]
fn forced_include_actor_entry_produces_user_message() {
    // Given an Actor entry with ForcedInclude (not pinned).
    use crate::protocol::ContextOverride;
    let entries =
        vec![ChatEntry::actor("src", "text").with_context_override(ContextOverride::ForcedInclude)];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then a User message is produced (forced-include overrides kind default).
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        LlmMessage::User {
            content: "[Actor: src] text".into(),
        }
    );
}

// --- End-to-end: dangling tool calls after hard cancel ---

#[test]
fn forced_exclude_dangling_tool_call_produces_valid_messages() {
    // Given a history with an empty Assistant and a dangling ToolCall (no ToolResult),
    // simulating hard cancel during tool execution.
    let mut entries = vec![
        ChatEntry::user("run it"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#),
    ];

    // Force-exclude the dangling entries (simulating force_exclude_dangling_tool_calls).
    entries[1].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        ChangeSource::Internal {
            label: "test".to_string(),
        },
    );
    entries[2].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        ChangeSource::Internal {
            label: "test".to_string(),
        },
    );
    entries[2].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_string(),
        },
    );

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then only the User message is produced - no dangling tool_calls.
    assert_eq!(
        messages.len(),
        1,
        "expected only User message, got: {messages:?}"
    );
    assert!(matches!(&messages[0], LlmMessage::User { .. }));

    // And no Assistant message with tool_calls exists.
    for msg in &messages {
        if let LlmMessage::Assistant { tool_calls, .. } = msg {
            assert!(
                tool_calls.as_ref().is_none_or(Vec::is_empty),
                "expected no tool_calls in assistant message, got: {tool_calls:?}"
            );
        }
    }
}

#[test]
fn forced_exclude_preserves_complete_tool_loop_in_messages() {
    // Given a history with a complete tool loop plus a dangling ToolCall.
    let mut entries = vec![
        ChatEntry::user("run it"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#),
        ChatEntry::tool_result("tc-1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc-2", "read", r#"{"file":"a.rs"}"#),
    ];

    // Force-exclude only the dangling entries (tc-2 and its empty Assistant).
    entries[4].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_string(),
        },
    );
    entries[5].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_string(),
        },
    );

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then the complete tool loop is preserved and the dangling one is excluded.
    // Expected: User, Assistant(tc-1), Tool(tc-1).
    assert_eq!(messages.len(), 3, "expected 3 messages, got: {messages:?}");
    assert!(matches!(&messages[0], LlmMessage::User { .. }));
    // Assistant with tc-1 tool_call.
    match &messages[1] {
        LlmMessage::Assistant { tool_calls, .. } => {
            let tc = tool_calls.as_ref().expect("expected tool_calls");
            assert_eq!(tc.len(), 1);
            assert_eq!(tc[0].id, "tc-1");
        }
        _ => panic!("expected Assistant message"),
    }
    // Tool result for tc-1.
    match &messages[2] {
        LlmMessage::Tool { tool_call_id, .. } => {
            assert_eq!(tool_call_id, "tc-1");
        }
        _ => panic!("expected Tool message"),
    }
}

#[test]
fn no_dangling_tool_calls_in_messages_after_hard_cancel() {
    // Given a complex history with multiple tool loops where some are dangling.
    let mut entries = vec![
        ChatEntry::user("do stuff"),
        ChatEntry::assistant("step 1"),
        ChatEntry::tool_call("tc-1", "bash", "ls"),
        ChatEntry::tool_result("tc-1", "bash", "out", ToolResultStatus::Success),
        ChatEntry::assistant("step 2"),
        ChatEntry::tool_call("tc-2", "bash", "cat"),
        ChatEntry::tool_result("tc-2", "bash", "contents", ToolResultStatus::Success),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc-3", "read", "a.rs"),
        ChatEntry::tool_call("tc-4", "bash", "pwd"),
    ];

    // Force-exclude the dangling entries (tc-3, tc-4, and their empty Assistant).
    entries[7].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_string(),
        },
    );
    entries[8].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_string(),
        },
    );
    entries[9].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_string(),
        },
    );

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then every Assistant message with tool_calls has a matching Tool message for each.
    let mut tool_call_ids: Vec<String> = Vec::new();
    let mut tool_result_ids: Vec<String> = Vec::new();

    for msg in &messages {
        match msg {
            LlmMessage::Assistant {
                tool_calls: Some(calls),
                ..
            } => {
                for tc in calls {
                    tool_call_ids.push(tc.id.clone());
                }
            }
            LlmMessage::Tool { tool_call_id, .. } => {
                tool_result_ids.push(tool_call_id.clone());
            }
            _ => {}
        }
    }

    // Every tool_call_id must have a matching tool_result_id.
    for tc_id in &tool_call_ids {
        assert!(
            tool_result_ids.iter().any(|r| r == tc_id),
            "dangling tool_call {tc_id} found in messages - no matching Tool result"
        );
    }
}

#[test]
fn complete_tool_batch_produces_valid_messages() {
    // Given a history simulating auto-compaction during ToolUse where the tool batch
    // completed normally (all tool calls have matching results).
    // This represents the state after: stream completed with ToolUse -> tools execute ->
    // auto-compaction consumed by on_tool_batch_completed -> session goes to Compacting.
    // The history has complete tool loops because the batch finished.
    let entries = vec![
        ChatEntry::user("fix this bug"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc-1", "bash", "ls"),
        ChatEntry::tool_result("tc-1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc-2", "read", "file.rs"),
        ChatEntry::tool_result("tc-2", "read", "contents", ToolResultStatus::Success),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then every Assistant message with tool_calls has a matching Tool message for each.
    let mut tool_call_ids: Vec<String> = Vec::new();
    let mut tool_result_ids: Vec<String> = Vec::new();

    for msg in &messages {
        match msg {
            LlmMessage::Assistant {
                tool_calls: Some(calls),
                ..
            } => {
                for tc in calls {
                    tool_call_ids.push(tc.id.clone());
                }
            }
            LlmMessage::Tool { tool_call_id, .. } => {
                tool_result_ids.push(tool_call_id.clone());
            }
            _ => {}
        }
    }

    // Every tool_call_id must have a matching tool_result_id.
    for tc_id in &tool_call_ids {
        assert!(
            tool_result_ids.iter().any(|r| r == tc_id),
            "dangling tool_call {tc_id} found in messages after auto-compaction with complete batch"
        );
    }
}

#[rstest::rstest]
fn orphan_tool_call_after_excluded_empty_assistant_creates_synthetic() {
    // Given an empty assistant (excluded by default) followed by a tool call
    // and a tool result - simulating the user having excluded entries.
    let entries = vec![
        ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#),
        ChatEntry::tool_result("tc-1", "bash", "file.txt", ToolResultStatus::Success),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then a synthetic empty assistant message is created for the orphan tool call.
    assert_eq!(messages.len(), 2);
    match &messages[0] {
        LlmMessage::Assistant {
            content,
            tool_calls,
        } => {
            assert!(content.is_empty());
            assert!(tool_calls.is_some());
            assert_eq!(tool_calls.as_ref().expect("some").len(), 1);
            assert_eq!(tool_calls.as_ref().expect("some")[0].id, "tc-1");
        }
        other => panic!("expected Assistant, got {other:?}"),
    }
    match &messages[1] {
        LlmMessage::Tool {
            tool_call_id,
            name,
            content,
        } => {
            assert_eq!(tool_call_id, "tc-1");
            assert_eq!(name, "bash");
            assert_eq!(content, "file.txt");
        }
        other => panic!("expected Tool, got {other:?}"),
    }
}
