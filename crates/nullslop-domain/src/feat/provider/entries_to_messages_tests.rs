#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::feat::provider::entries_to_messages::entries_to_messages;
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

#[rstest::rstest]
fn entries_to_messages_skips_thinking_entries() {
    // Given a history with thinking, user, and assistant entries.
    let entries = vec![
        ChatEntry::thinking("reasoning here"),
        ChatEntry::user("hello"),
        ChatEntry::assistant("hi"),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then thinking is excluded, producing only user and assistant.
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
fn skill_entry_produces_system_message_with_xml() {
    // Given a skill entry.
    let entries = vec![ChatEntry::skill(
        "web-coder",
        "/home/user/.agents/skills/web-coder/SKILL.md",
        "Expert web development skill.",
    )];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then a System message is produced with the skill XML format.
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        LlmMessage::System {
            content: "<skill name=\"web-coder\" location=\"/home/user/.agents/skills/web-coder/SKILL.md\">\nExpert web development skill.\n</skill>".to_owned(),
        }
    );
}

#[rstest::rstest]
fn info_entries_are_skipped() {
    // Given an Info entry alongside user and assistant entries.
    let entries = vec![
        ChatEntry::info(vec![ratatui::text::Line::from("welcome")]),
        ChatEntry::user("hello"),
        ChatEntry::assistant("hi"),
    ];

    // When converting to messages.
    let messages = entries_to_messages(&entries);

    // Then Info is excluded, producing only user and assistant.
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
            entries_compacted: 10,
            model_used: "test/model".to_owned(),
        },
        pin_position: None,
        ignored: false,
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
                entries_compacted: 2,
                model_used: "test/model".to_owned(),
            },
            pin_position: None,
            ignored: false,
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
