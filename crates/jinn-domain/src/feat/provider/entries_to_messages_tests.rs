#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

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
            content: "hello".into(),
            attachments: Vec::new(),
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
            content: "hello".into(),
            attachments: Vec::new(),
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
            attachments: Vec::new(),
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
            attachments: Vec::new(),
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
            attachments: Vec::new(),
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
            content: "hello".into(),
            attachments: Vec::new(),
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
            content: "hello".into(),
            attachments: Vec::new(),
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
        timing: crate::protocol::EntryTiming::instant_now(),
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
        degraded_paths: None,
    }];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then a User message with the wrapped summary is produced.
    assert_eq!(messages.len(), 1);
    let content = match &messages[0] {
        LlmMessage::User { content, .. } => content.clone(),
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
            content: "important".into(),
            attachments: Vec::new(),
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
            content: "second".into(),
            attachments: Vec::new(),
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
            timing: crate::protocol::EntryTiming::instant_now(),
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
            degraded_paths: None,
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
        matches!(&messages[0], LlmMessage::User { content, .. } if content.contains("The user asked about X"))
    );
    // Recent entries follow.
    assert_eq!(
        messages[1],
        LlmMessage::User {
            content: "new question".into(),
            attachments: Vec::new(),
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
            content: "hello".into(),
            attachments: Vec::new(),
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
            content:
                "The user has shared the following output for you to address:\n\nimportant error"
                    .into(),
            attachments: Vec::new(),
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
            content: "[Error] pinned error".into(),
            attachments: Vec::new(),
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
            content: "[Thinking] reasoning".into(),
            attachments: Vec::new(),
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
            content: "[Transient] welcome".into(),
            attachments: Vec::new(),
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
            attachments: Vec::new(),
        }
    );
}

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
            label: "test".to_owned(),
        },
    );
    entries[2].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        ChangeSource::Internal {
            label: "test".to_owned(),
        },
    );
    entries[2].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_owned(),
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
            label: "test".to_owned(),
        },
    );
    entries[5].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_owned(),
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
            label: "test".to_owned(),
        },
    );
    entries[8].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_owned(),
        },
    );
    entries[9].apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        crate::feat::session::chat_entry::ChangeSource::Internal {
            label: "test".to_owned(),
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

// ═══════════════════════════════════════════════════════════════════════════
// Compaction boundary: excluding the summary must never break message sequencing.
// ═══════════════════════════════════════════════════════════════════════════

/// Helper: every Assistant tool_call id has a following Tool message, no tool
/// message precedes its assistant, and the first message is a valid opener
/// (User or System).
fn assert_message_sequence_is_valid(messages: &[LlmMessage]) {
    // The first message must be a standalone opener, never a Tool or an
    // Assistant that is empty / has tool_calls (those need a preceding turn).
    assert!(
        matches!(
            messages.first(),
            Some(LlmMessage::User { .. } | LlmMessage::System { .. })
        ),
        "first message must be a User or System opener, got {:?}",
        messages.first()
    );

    let mut tool_call_ids: Vec<String> = Vec::new();
    let mut tool_result_ids: Vec<String> = Vec::new();
    for msg in messages {
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

    // Every emitted tool_call must have a matching tool result.
    for tc_id in &tool_call_ids {
        assert!(
            tool_result_ids.iter().any(|r| r == tc_id),
            "dangling tool_call {tc_id} found - no matching Tool result"
        );
    }
    // And no orphan tool result whose call was dropped.
    for tr_id in &tool_result_ids {
        assert!(
            tool_call_ids.iter().any(|r| r == tr_id),
            "orphan tool result {tr_id} found - no preceding tool_call"
        );
    }
}

fn force_exclude(entry: &mut ChatEntry) {
    entry.apply_context_override(
        crate::protocol::ContextOverride::ForcedExclude,
        ChangeSource::Internal {
            label: "test".to_owned(),
        },
    );
}

fn compaction_entry(summary: &str) -> ChatEntry {
    use crate::protocol::{ChatEntryId, ChatEntryKind, EntryTiming};
    ChatEntry {
        id: ChatEntryId::new(),
        timing: EntryTiming::instant_now(),
        kind: ChatEntryKind::Compaction {
            summary: summary.to_owned(),
            tokens_before: 100,
            tokens_after: 50,
            entries_compacted: 5,
            model_used: "test-model".to_owned(),
        },
        pin_position: None,
        context_override: crate::protocol::ContextOverride::Default,
        context_history: Vec::new(),
        degraded_paths: None,
    }
}

#[test]
fn excluding_compaction_summary_never_breaks_message_sequencing() {
    // Given a history whose compaction boundary would, without Pass 3, leave
    // an Assistant opener as the first kept entry:
    //   [User, Assistant(BIG), Assistant(opener), User(recent turn)]
    // After adjust_cut_to_boundary the kept region must open with the final User.
    use crate::feat::compaction_worker::algorithm::adjust_cut_to_boundary;

    let mut entries = vec![
        ChatEntry::user("start"),
        ChatEntry::assistant(format!("big {}", "w".repeat(600))),
        ChatEntry::assistant("recent opener"),
        ChatEntry::user("recent turn"),
    ];

    let cut = adjust_cut_to_boundary(&entries, 2);

    // Force-exclude everything on the compacted side (indices < cut) and
    // insert + force-exclude the compaction summary at the boundary.
    for entry in entries.iter_mut().take(cut) {
        force_exclude(entry);
    }
    let mut summary = compaction_entry("summary");
    force_exclude(&mut summary);
    entries.push(summary);

    // When converting the kept region (summary force-excluded) to messages.
    let messages = entries_to_messages(&entries);

    // Then the sequence is structurally valid despite the summary being excluded.
    assert!(
        !messages.is_empty(),
        "kept region should produce at least one message"
    );
    assert_message_sequence_is_valid(&messages);
}

#[test]
fn including_compaction_summary_produces_valid_sequencing() {
    // Same history as above, but the summary is included (regression: the
    // old code was only valid because the summary masked the broken opener).
    use crate::feat::compaction_worker::algorithm::adjust_cut_to_boundary;

    let mut entries = vec![
        ChatEntry::user("start"),
        ChatEntry::assistant(format!("big {}", "w".repeat(600))),
        ChatEntry::assistant("recent opener"),
        ChatEntry::user("recent turn"),
    ];

    let cut = adjust_cut_to_boundary(&entries, 2);
    for entry in entries.iter_mut().take(cut) {
        force_exclude(entry);
    }
    let summary = compaction_entry("summary");
    entries.push(summary);

    // When converting with the summary included.
    let messages = entries_to_messages(&entries);

    // Then the sequence is valid.
    assert_message_sequence_is_valid(&messages);
}

/// Assert that a message sequence is structurally valid as a standalone
/// conversation: the first message is a valid opener (User or System), and
/// every assistant tool_call has a matching following Tool message with no
/// dangling tool_call_id.
fn assert_messages_are_structurally_valid(messages: &[LlmMessage]) {
    // The first message must be a valid conversation opener.
    assert!(
        matches!(
            messages.first(),
            Some(LlmMessage::User { .. } | LlmMessage::System { .. })
        ),
        "first message must be a User or System turn, got {:?}",
        messages.first()
    );

    // Collect every tool_call_id emitted by an assistant, and every tool_call_id
    // resolved by a Tool message.
    let mut tool_call_ids: Vec<String> = Vec::new();
    let mut tool_result_ids: Vec<String> = Vec::new();
    for msg in messages {
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

    for tc_id in &tool_call_ids {
        assert!(
            tool_result_ids.iter().any(|r| r == tc_id),
            "dangling tool_call {tc_id} has no matching Tool result"
        );
    }
}

#[test]
fn excluding_compaction_summary_yields_valid_message_sequence() {
    // Given a history reproducing the bug layout: a complete tool loop whose
    // summary-compaction would sit between a ToolResult and an Assistant, and
    // whose reserve boundary lands on an Assistant opener.
    //   [User, Assistant(big), ToolCall, ToolResult, Assistant(opener), User(recent)]
    use crate::feat::compaction_worker::algorithm::adjust_cut_to_boundary;
    use crate::feat::session::chat_entry::ChangeSource;

    let big_padding = "w".repeat(600);
    let mut entries = vec![
        ChatEntry::user("start"),
        ChatEntry::assistant(format!("big {big_padding}")),
        ChatEntry::tool_call("tc-1", "bash", "ls"),
        ChatEntry::tool_result("tc-1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("recent opener"),
        ChatEntry::user("recent turn"),
    ];

    // When compaction computes its cut boundary.
    // With a small reserve, the cut lands on the Assistant opener (index 4);
    // Pass 3 must advance it to the User at index 5 so the kept region is valid.
    let cut = adjust_cut_to_boundary(&entries, 4);
    assert_eq!(
        cut, 5,
        "cut must advance past the Assistant opener to the User"
    );

    // Force-exclude every entry on the compacted side (indices < cut).
    for entry in entries.iter_mut().take(cut) {
        entry.apply_context_override(
            crate::protocol::ContextOverride::ForcedExclude,
            ChangeSource::Internal {
                label: "compaction".to_owned(),
            },
        );
    }

    // Simulate excluding the compaction summary itself: build the kept region
    // (indices >= cut) WITHOUT inserting any summary entry.
    let kept: Vec<ChatEntry> = entries.iter().skip(cut).cloned().collect();

    // Then the kept region converts to a structurally valid message sequence.
    let messages = entries_to_messages(&kept);
    assert!(
        !messages.is_empty(),
        "kept region must produce at least one message"
    );
    assert_messages_are_structurally_valid(&messages);
}

#[rstest::rstest]
fn entries_to_messages_passes_user_attachments_through() {
    use jinn_provider::Attachment;

    // Given a user entry with one image attachment.
    let mut entry = ChatEntry::user("describe this");
    if let crate::protocol::ChatEntryKind::User { attachments, .. } = &mut entry.kind {
        attachments.push(Attachment::image("image/png", vec![1, 2, 3]));
    }
    let entries = vec![entry];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then the User message carries the attachment.
    assert_eq!(messages.len(), 1);
    let LlmMessage::User {
        content,
        attachments,
    } = &messages[0]
    else {
        panic!("expected a User message");
    };
    assert_eq!(content, "describe this");
    assert_eq!(attachments.len(), 1);
    assert!(attachments[0].is_image());
}
