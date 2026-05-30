#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use super::chat_entry::*;
use super::tool_result_status::ToolResultStatus;

#[rstest::rstest]
fn chat_entry_id_is_unique() {
    // Given two generated IDs.
    let id1 = ChatEntryId::new();
    let id2 = ChatEntryId::new();

    // Then they are not equal.
    assert_ne!(id1, id2);
}

#[rstest::rstest]
fn chat_entry_id_is_valid_uuid() {
    // Given a generated ID.
    let id = ChatEntryId::new();

    // Then the string representation is a valid UUID.
    let s = id.to_string();
    assert!(uuid::Uuid::parse_str(&s).is_ok());
}

#[rstest::rstest]
fn user_entry_has_user_kind() {
    // Given text "hello".
    let text = "hello";

    // When creating a user entry.
    let entry = ChatEntry::user(text);

    // Then kind is User("hello").
    assert_eq!(
        entry.kind,
        ChatEntryKind::User {
            display: "hello".to_owned(),
            expanded: "hello".to_owned(),
        }
    );
}

#[rstest::rstest]
fn system_entry_has_system_kind() {
    // Given text "ready".
    let text = "ready";

    // When creating a system entry.
    let entry = ChatEntry::system(text);

    // Then kind is System("ready").
    assert_eq!(entry.kind, ChatEntryKind::System("ready".to_owned()));
}

#[rstest::rstest]
fn entry_has_timestamp() {
    // Given the current time.
    let before = jiff::Timestamp::now();

    // When creating a user entry.
    let entry = ChatEntry::user("test");

    // Then the timestamp is close to now.
    let after = jiff::Timestamp::now();
    assert!(entry.timestamp >= before);
    assert!(entry.timestamp <= after);
}

#[rstest::rstest]
fn assistant_entry_has_assistant_kind() {
    // Given text "hello".
    let text = "hello";

    // When creating an assistant entry.
    let entry = ChatEntry::assistant(text);

    // Then kind is Assistant("hello").
    assert_eq!(entry.kind, ChatEntryKind::Assistant("hello".to_owned()));
}

#[rstest::rstest]
fn actor_entry_has_actor_kind() {
    // Given source "echo" and text "HELLO".
    let source = "echo";
    let text = "HELLO";

    // When creating an actor entry.
    let entry = ChatEntry::actor(source, text);

    // Then kind is Actor with correct source and text.
    assert_eq!(
        entry.kind,
        ChatEntryKind::Actor {
            source: "echo".to_owned(),
            text: "HELLO".to_owned(),
        }
    );
}

#[rstest::rstest]
fn tool_call_entry_has_tool_call_kind() {
    // Given tool call details.
    let id = "call_123";
    let name = "echo";
    let arguments = r#"{"input":"hi"}"#;

    // When creating a tool call entry.
    let entry = ChatEntry::tool_call(id, name, arguments);

    // Then kind is ToolCall with correct fields.
    assert_eq!(
        entry.kind,
        ChatEntryKind::ToolCall {
            id: "call_123".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        }
    );
}

#[rstest::rstest]
fn tool_result_entry_has_tool_result_kind() {
    // Given tool result details.
    let id = "call_123";
    let name = "echo";
    let content = "hi";

    // When creating a tool result entry.
    let entry = ChatEntry::tool_result(id, name, content, ToolResultStatus::Success);

    // Then kind is ToolResult with correct fields.
    assert_eq!(
        entry.kind,
        ChatEntryKind::ToolResult {
            id: "call_123".to_owned(),
            name: "echo".to_owned(),
            content: "hi".to_owned(),
            status: ToolResultStatus::Success,
            full_content: None,
            truncation: None,
        }
    );
}

// --- PinPosition tests ---

#[rstest::rstest]
#[case::user(ChatEntry::user("u"))]
#[case::system(ChatEntry::system("s"))]
#[case::assistant(ChatEntry::assistant("a"))]
#[case::actor(ChatEntry::actor("src", "t"))]
#[case::tool_call(ChatEntry::tool_call("id", "name", "args"))]
#[case::tool_result(ChatEntry::tool_result("id", "name", "content", ToolResultStatus::Success))]
#[case::skill(ChatEntry::skill("name", "/path", "content"))]
#[case::transient(ChatEntry::transient("info"))]
fn pin_position_defaults_to_none(#[case] entry: ChatEntry) {
    // Given an entry created with any ChatEntry constructor.
    // When checking pin_position.
    // Then pin_position is None by default.
    assert_eq!(entry.pin_position, None);
}

#[rstest::rstest]
fn with_pin_sets_position() {
    // Given a user entry.
    let entry = ChatEntry::user("test").with_pin(PinPosition::Top);

    // Then pin_position is Some(Top).
    assert_eq!(entry.pin_position, Some(PinPosition::Top));
}

#[rstest::rstest]
fn is_pinned_returns_true_when_pinned() {
    // Given a pinned entry.
    let entry = ChatEntry::user("test").with_pin(PinPosition::Top);

    // Then is_pinned returns true.
    assert!(entry.is_pinned());
}

#[rstest::rstest]
fn is_pinned_returns_false_when_unpinned() {
    // Given a default entry.
    let entry = ChatEntry::user("test");

    // Then is_pinned returns false.
    assert!(!entry.is_pinned());
}

#[rstest::rstest]
fn pin_position_returns_some_when_pinned() {
    // Given a pinned entry.
    let entry = ChatEntry::user("test").with_pin(PinPosition::Bottom);

    // Then pin_position() returns the correct variant.
    assert_eq!(entry.pin_position(), Some(PinPosition::Bottom));
}

#[rstest::rstest]
fn pin_position_returns_none_when_unpinned() {
    // Given an unpinned entry.
    let entry = ChatEntry::user("test");

    // Then pin_position() returns None.
    assert_eq!(entry.pin_position(), None);
}

#[rstest::rstest]
fn pin_position_deserializes_old_format() {
    // Given JSON without pin_position field (old format).
    let json = r#"{"id":"550e8400-e29b-41d4-a716-446655440000","timestamp":"2024-01-01T00:00:00Z","kind":{"User":{"display":"hello","expanded":"hello"}}}"#;

    // When deserializing.
    let entry: ChatEntry = serde_json::from_str(json).expect("deserialize");

    // Then pin_position is None (backward compat via #[serde(default)]).
    assert_eq!(entry.pin_position, None);
}

// --- Thinking entry tests ---

#[rstest::rstest]
fn thinking_entry_has_thinking_kind() {
    // Given text "reasoning here".
    let text = "reasoning here";

    // When creating a thinking entry.
    let entry = ChatEntry::thinking(text);

    // Then kind is Thinking("reasoning here").
    assert_eq!(
        entry.kind,
        ChatEntryKind::Thinking("reasoning here".to_owned())
    );
}

#[rstest::rstest]
fn thinking_kind_str_returns_thinking() {
    // Given a thinking entry.
    let entry = ChatEntry::thinking("test");

    // Then kind_str returns "thinking".
    assert_eq!(entry.kind_str(), "thinking");
}

#[rstest::rstest]
fn thinking_text_returns_content() {
    // Given a thinking entry.
    let entry = ChatEntry::thinking("some reasoning");

    // Then text() returns the reasoning text.
    assert_eq!(entry.text(), "some reasoning");
}

#[rstest::rstest]
fn thinking_entry_serializes_roundtrip() {
    // Given a thinking entry.
    let entry = ChatEntry::thinking("I need to think about this");

    // When serializing and deserializing.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then the roundtrip preserves the kind.
    assert_eq!(
        back.kind,
        ChatEntryKind::Thinking("I need to think about this".to_owned())
    );
}

#[rstest::rstest]
fn thinking_entry_pin_position_defaults_to_none() {
    // Given a thinking entry.
    let entry = ChatEntry::thinking("test");

    // Then pin_position is None.
    assert_eq!(entry.pin_position, None);
}

// --- Skill entry tests ---

#[rstest::rstest]
fn skill_entry_has_skill_kind() {
    // Given skill details.
    let name = "web-coder";
    let location = "/home/user/.agents/skills/web-coder/SKILL.md";
    let content = "# Web Coder\n\nExpert web development skill.";

    // When creating a skill entry.
    let entry = ChatEntry::skill(name, location, content);

    // Then kind is Skill with correct fields.
    assert_eq!(
        entry.kind,
        ChatEntryKind::Skill {
            name: "web-coder".to_owned(),
            location: "/home/user/.agents/skills/web-coder/SKILL.md".to_owned(),
            content: "# Web Coder\n\nExpert web development skill.".to_owned(),
        }
    );
}

#[rstest::rstest]
fn skill_kind_str_returns_skill() {
    // Given a skill entry.
    let entry = ChatEntry::skill("test", "/path", "content");

    // Then kind_str returns "skill".
    assert_eq!(entry.kind_str(), "skill");
}

#[rstest::rstest]
fn skill_text_returns_content() {
    // Given a skill entry.
    let entry = ChatEntry::skill("test", "/path", "skill body text");

    // Then text() returns the content.
    assert_eq!(entry.text(), "skill body text");
}

#[rstest::rstest]
fn skill_entry_serializes_roundtrip() {
    // Given a skill entry.
    let entry = ChatEntry::skill("my-skill", "/path/SKILL.md", "Skill content here");

    // When serializing and deserializing.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then the roundtrip preserves the kind.
    assert_eq!(
        back.kind,
        ChatEntryKind::Skill {
            name: "my-skill".to_owned(),
            location: "/path/SKILL.md".to_owned(),
            content: "Skill content here".to_owned(),
        }
    );
}

#[rstest::rstest]
fn skill_entry_pin_position_defaults_to_none() {
    // Given a skill entry.
    let entry = ChatEntry::skill("test", "/path", "content");

    // Then pin_position is None.
    assert_eq!(entry.pin_position, None);
}

#[rstest::rstest]
fn skill_entry_can_be_pinned() {
    // Given a skill entry pinned to TOP.
    let entry = ChatEntry::skill("test", "/path", "content").with_pin(PinPosition::Top);

    // Then it is pinned with TOP position.
    assert!(entry.is_pinned());
    assert_eq!(entry.pin_position(), Some(PinPosition::Top));
}

#[rstest::rstest]
fn skill_entry_serializes_correct_json_shape() {
    // Given a skill entry.
    let entry = ChatEntry::skill("test-skill", "/skills/test-skill/SKILL.md", "body");

    // When serializing.
    let json = serde_json::to_string(&entry.kind).expect("serialize");

    // Then the JSON has the expected shape: {"Skill": {...}}.
    let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert!(v.get("Skill").is_some(), "should have Skill key");
    let skill = &v["Skill"];
    assert_eq!(skill["name"], "test-skill");
    assert_eq!(skill["location"], "/skills/test-skill/SKILL.md");
    assert_eq!(skill["content"], "body");
}

// --- Transient entry tests ---

#[rstest::rstest]
fn transient_entry_has_transient_kind() {
    // Given text "welcome".
    let text = "welcome";

    // When creating a transient entry.
    let entry = ChatEntry::transient(text);

    // Then kind is Transient.
    assert!(matches!(entry.kind, ChatEntryKind::Transient(_)));
    // And text() returns the text.
    assert_eq!(entry.text(), "welcome");
}

#[rstest::rstest]
fn transient_kind_str_returns_transient() {
    // Given a transient entry.
    let entry = ChatEntry::transient("test");
    assert_eq!(entry.kind_str(), "transient");
}

#[rstest::rstest]
fn transient_text_returns_content() {
    // Given a transient entry.
    let entry = ChatEntry::transient("some hint");

    // Then text() returns the content.
    assert_eq!(entry.text(), "some hint");
}

#[rstest::rstest]
fn transient_entry_serializes_roundtrip() {
    // Given a transient entry.
    let entry = ChatEntry::transient("Welcome to jinn!");

    // When serializing and deserializing.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then the roundtrip converts Transient to System (Transient entries are not persisted).
    assert_eq!(
        back.kind,
        ChatEntryKind::System("Welcome to jinn!".to_owned())
    );
}

#[rstest::rstest]
fn transient_entry_pin_position_defaults_to_none() {
    // Given a transient entry.
    let entry = ChatEntry::transient("test");

    // Then pin_position is None.
    assert_eq!(entry.pin_position, None);
}

#[rstest::rstest]
fn transient_entry_serializes_correct_json_shape() {
    // Given a transient entry.
    let entry = ChatEntry::transient("some info");

    // When serializing just the kind.
    let json = serde_json::to_string(&entry.kind).expect("serialize");

    // Then the JSON has the expected shape: {"Transient": "..."}.
    let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert!(v.get("Transient").is_some(), "should have Transient key");
    assert_eq!(v["Transient"], "some info");
}

// --- ToolResultStatus serialization tests ---

#[rstest::rstest]
fn tool_result_status_pending_serializes() {
    // Given a ToolResult entry with Pending status.
    let entry = ChatEntry::tool_result("id", "bash", "", ToolResultStatus::Pending);

    // When serializing and deserializing.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then the status is preserved.
    assert_eq!(back.kind, entry.kind);
}

#[rstest::rstest]
fn tool_result_status_success_serializes() {
    // Given a ToolResult entry with Success status.
    let entry = ChatEntry::tool_result("id", "bash", "ok", ToolResultStatus::Success);

    // When serializing and deserializing.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then the status is preserved.
    assert_eq!(back.kind, entry.kind);
}

#[rstest::rstest]
fn tool_result_status_failure_serializes() {
    // Given a ToolResult entry with Failure status.
    let entry = ChatEntry::tool_result("id", "bash", "err", ToolResultStatus::Failure);

    // When serializing and deserializing.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then the status is preserved.
    assert_eq!(back.kind, entry.kind);
}

#[rstest::rstest]
fn tool_result_deserializes_old_success_true_format() {
    // Given JSON in the old format with success: true.
    let json = r#"{"id":"550e8400-e29b-41d4-a716-446655440000","timestamp":"2024-01-01T00:00:00Z","kind":{"ToolResult":{"id":"call_1","name":"bash","content":"ok","success":true}}}"#;

    // When deserializing.
    let entry: ChatEntry = serde_json::from_str(json).expect("deserialize");

    // Then it maps to Success status.
    assert_eq!(
        entry.kind,
        ChatEntryKind::ToolResult {
            id: "call_1".to_owned(),
            name: "bash".to_owned(),
            content: "ok".to_owned(),
            status: ToolResultStatus::Success,
            full_content: None,
            truncation: None,
        }
    );
}

#[rstest::rstest]
fn tool_result_deserializes_old_success_false_format() {
    // Given JSON in the old format with success: false.
    let json = r#"{"id":"550e8400-e29b-41d4-a716-446655440000","timestamp":"2024-01-01T00:00:00Z","kind":{"ToolResult":{"id":"call_1","name":"bash","content":"err","success":false}}}"#;

    // When deserializing.
    let entry: ChatEntry = serde_json::from_str(json).expect("deserialize");

    // Then it maps to Failure status.
    assert_eq!(
        entry.kind,
        ChatEntryKind::ToolResult {
            id: "call_1".to_owned(),
            name: "bash".to_owned(),
            content: "err".to_owned(),
            status: ToolResultStatus::Failure,
            full_content: None,
            truncation: None,
        }
    );
}

#[rstest::rstest]
fn pending_tool_result_fingerprint_includes_content() {
    // Given two pending ToolResult entries with different content.
    let entry1 = ChatEntry::tool_result("id", "bash", "line1", ToolResultStatus::Pending);
    let entry2 = ChatEntry::tool_result("id", "bash", "line1\nline2", ToolResultStatus::Pending);

    // Then their fingerprints differ (content included for all statuses).
    assert_ne!(entry1.content_fingerprint(), entry2.content_fingerprint());
}

#[rstest::rstest]
fn completed_tool_result_fingerprint_includes_content() {
    // Given two completed ToolResult entries with different content.
    let entry1 = ChatEntry::tool_result("id", "bash", "line1", ToolResultStatus::Success);
    let entry2 = ChatEntry::tool_result("id", "bash", "line1\nline2", ToolResultStatus::Success);

    // Then their fingerprints differ (content included for completed).
    assert_ne!(entry1.content_fingerprint(), entry2.content_fingerprint());
}

#[rstest::rstest]
fn tool_result_fingerprint_differs_with_truncation() {
    // Given two completed ToolResult entries with same content but different truncation.
    let entry1 = ChatEntry::tool_result("id", "bash", "line1", ToolResultStatus::Success);
    let entry2 = ChatEntry::tool_result_truncated(
        "id",
        "bash",
        "line1".to_owned(),
        "line1\nline2".to_owned(),
        ToolResultStatus::Success,
        jinn_provider::tool_types::TruncationMeta {
            truncated_by: jinn_provider::tool_types::TruncatedBy::Lines,
            total_lines: 2,
            total_bytes: 19,
            output_lines: 1,
            output_bytes: 9,
        },
    );

    // Then their fingerprints differ (truncation presence affects hash).
    assert_ne!(entry1.content_fingerprint(), entry2.content_fingerprint());
}

// --- is_empty_assistant tests ---

#[rstest::rstest]
fn is_empty_assistant_true_for_empty_assistant() {
    // Given an empty assistant entry.
    let entry = ChatEntry::assistant("");

    // Then is_empty_assistant returns true.
    assert!(entry.is_empty_assistant());
}

#[rstest::rstest]
fn is_empty_assistant_false_for_nonempty_assistant() {
    // Given a non-empty assistant entry.
    let entry = ChatEntry::assistant("hello");

    // Then is_empty_assistant returns false.
    assert!(!entry.is_empty_assistant());
}

#[rstest::rstest]
fn is_empty_assistant_false_for_user_entry() {
    // Given a user entry (even with empty display text).
    let entry = ChatEntry::user("");

    // Then is_empty_assistant returns false.
    assert!(!entry.is_empty_assistant());
}

#[rstest::rstest]
fn is_empty_assistant_false_for_system_entry() {
    // Given a system entry.
    let entry = ChatEntry::system("");

    // Then is_empty_assistant returns false.
    assert!(!entry.is_empty_assistant());
}

// --- is_included_by_default tests ---

#[rstest::rstest]
fn user_kind_is_included_by_default() {
    // Given a User entry.
    let entry = ChatEntry::user("hello");

    // Then the kind is included by default.
    assert!(entry.kind.is_included_by_default());
}

#[rstest::rstest]
fn assistant_kind_is_included_by_default() {
    // Given an Assistant entry.
    let entry = ChatEntry::assistant("response");

    // Then the kind is included by default.
    assert!(entry.kind.is_included_by_default());
}

#[rstest::rstest]
fn error_kind_is_not_included_by_default() {
    // Given an Error entry.
    let entry = ChatEntry::error("something went wrong");

    // Then the kind is NOT included by default.
    assert!(!entry.kind.is_included_by_default());
}

#[rstest::rstest]
fn tool_call_kind_is_included_by_default() {
    // Given a ToolCall entry.
    let entry = ChatEntry::tool_call("id", "name", "{}");

    // Then the kind is included by default.
    assert!(entry.kind.is_included_by_default());
}

#[rstest::rstest]
fn tool_result_kind_is_included_by_default() {
    // Given a ToolResult entry.
    let entry = ChatEntry::tool_result("id", "name", "content", ToolResultStatus::Success);

    // Then the kind is included by default.
    assert!(entry.kind.is_included_by_default());
}

#[rstest::rstest]
fn skill_kind_is_included_by_default() {
    // Given a Skill entry.
    let entry = ChatEntry::skill("name", "/path", "content");

    // Then the kind is included by default.
    assert!(entry.kind.is_included_by_default());
}

#[rstest::rstest]
fn compaction_kind_is_included_by_default() {
    // Given a Compaction entry.
    let kind = ChatEntryKind::Compaction {
        summary: "summary".to_owned(),
        tokens_before: 100,
        tokens_after: 50,
        entries_compacted: 5,
        model_used: "gpt-4".to_owned(),
    };

    // Then the kind is included by default.
    assert!(kind.is_included_by_default());
}

#[rstest::rstest]
fn thinking_kind_is_not_included_by_default() {
    // Given a Thinking entry.
    let entry = ChatEntry::thinking("reasoning");

    // Then the kind is NOT included by default.
    assert!(!entry.kind.is_included_by_default());
}

#[rstest::rstest]
fn transient_kind_is_not_included_by_default() {
    // Given a Transient entry.
    let entry = ChatEntry::transient("hint");

    // Then the kind is NOT included by default.
    assert!(!entry.kind.is_included_by_default());
}

#[rstest::rstest]
fn system_kind_is_not_included_by_default() {
    // Given a System entry.
    let entry = ChatEntry::system("status");

    // Then the kind is NOT included by default.
    assert!(!entry.kind.is_included_by_default());
}

#[rstest::rstest]
fn actor_kind_is_not_included_by_default() {
    // Given an Actor entry.
    let entry = ChatEntry::actor("echo", "HELLO");

    // Then the kind is NOT included by default.
    assert!(!entry.kind.is_included_by_default());
}

// --- is_in_context tests ---

#[rstest::rstest]
fn user_entry_is_in_context_by_default() {
    // Given a default User entry.
    let entry = ChatEntry::user("hello");

    // Then it is in context.
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn thinking_entry_is_not_in_context_by_default() {
    // Given a default Thinking entry.
    let entry = ChatEntry::thinking("reasoning");

    // Then it is NOT in context.
    assert!(!entry.is_in_context());
}

#[rstest::rstest]
fn system_entry_is_not_in_context_by_default() {
    // Given a default System entry.
    let entry = ChatEntry::system("status");

    // Then it is NOT in context.
    assert!(!entry.is_in_context());
}

#[rstest::rstest]
fn transient_entry_is_not_in_context_by_default() {
    // Given a default Transient entry.
    let entry = ChatEntry::transient("hint");

    // Then it is NOT in context.
    assert!(!entry.is_in_context());
}

#[rstest::rstest]
fn actor_entry_is_not_in_context_by_default() {
    // Given a default Actor entry.
    let entry = ChatEntry::actor("echo", "HELLO");

    // Then it is NOT in context.
    assert!(!entry.is_in_context());
}

#[rstest::rstest]
fn pinned_thinking_entry_is_in_context() {
    // Given a Thinking entry pinned to Top.
    let entry = ChatEntry::thinking("reasoning").with_pin(PinPosition::Top);

    // Then pin overrides kind default — it IS in context.
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn pinned_system_entry_is_in_context() {
    // Given a System entry pinned to Top.
    let entry = ChatEntry::system("instruction").with_pin(PinPosition::Top);

    // Then pin overrides kind default — it IS in context.
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn ignored_user_entry_is_not_in_context() {
    // Given a User entry marked as ignored.
    let entry = ChatEntry::user("hello").with_ignored(true);

    // Then it is NOT in context.
    assert!(!entry.is_in_context());
}

#[rstest::rstest]
fn ignored_but_pinned_entry_is_in_context() {
    // Given a User entry that is both ignored and pinned.
    let entry = ChatEntry::user("hello")
        .with_ignored(true)
        .with_pin(PinPosition::Top);

    // Then pin overrides ignore — it IS in context.
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn pinned_ignored_thinking_entry_is_in_context() {
    // Given a Thinking entry that is both ignored and pinned.
    let entry = ChatEntry::thinking("reason")
        .with_ignored(true)
        .with_pin(PinPosition::Bottom);

    // Then pin overrides both kind default and ignore — it IS in context.
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn all_include_default_kinds_are_in_context() {
    // Given entries of all include-by-default kinds.
    let entries = vec![
        ChatEntry::user("hello"),
        ChatEntry::assistant("response"),
        ChatEntry::tool_call("id", "name", "{}"),
        ChatEntry::tool_result("id", "name", "content", ToolResultStatus::Success),
        ChatEntry::skill("name", "/path", "content"),
    ];

    // Then all are in context by default.
    for entry in &entries {
        assert!(
            entry.is_in_context(),
            "{} should be in context",
            entry.kind_str()
        );
    }
}

#[rstest::rstest]
fn all_exclude_default_kinds_are_not_in_context() {
    // Given entries of all exclude-by-default kinds.
    let entries = vec![
        ChatEntry::error("error"),
        ChatEntry::thinking("reasoning"),
        ChatEntry::transient("hint"),
        ChatEntry::system("status"),
        ChatEntry::actor("echo", "HELLO"),
    ];

    // Then none are in context by default.
    for entry in &entries {
        assert!(
            !entry.is_in_context(),
            "{} should NOT be in context",
            entry.kind_str()
        );
    }
}

// --- Empty assistant is_in_context tests ---

#[rstest::rstest]
fn empty_assistant_default_is_not_in_context() {
    // Given an empty Assistant entry with Default override.
    let entry = ChatEntry::assistant("");

    // Then it is NOT in context (empty assistants carry no information).
    assert!(!entry.is_in_context());
}

#[rstest::rstest]
fn nonempty_assistant_default_is_in_context() {
    // Given a non-empty Assistant entry with Default override.
    let entry = ChatEntry::assistant("response text");

    // Then it IS in context.
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn empty_assistant_forced_include_is_in_context() {
    // Given an empty Assistant entry with ForcedInclude override.
    let entry =
        ChatEntry::assistant("").with_context_override(ContextOverride::ForcedInclude);

    // Then ForcedInclude overrides the empty-assistant rule — it IS in context.
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn empty_assistant_forced_exclude_is_not_in_context() {
    // Given an empty Assistant entry with ForcedExclude override.
    let entry =
        ChatEntry::assistant("").with_context_override(ContextOverride::ForcedExclude);

    // Then it is NOT in context.
    assert!(!entry.is_in_context());
}

#[rstest::rstest]
fn pinned_empty_assistant_default_is_in_context() {
    // Given an empty Assistant entry that is pinned.
    let entry = ChatEntry::assistant("").with_pin(PinPosition::Top);

    // Then pin overrides the empty-assistant rule — it IS in context.
    assert!(entry.is_in_context());
}

// --- Pending tool result is_in_context tests ---

#[rstest::rstest]
fn pending_tool_result_default_is_not_in_context() {
    // Given a ToolResult with Pending status and Default override.
    let entry = ChatEntry::tool_result("tc-1", "bash", "", ToolResultStatus::Pending);

    // Then it is NOT in context (pending results are incomplete).
    assert!(!entry.is_in_context());
}

#[rstest::rstest]
fn success_tool_result_default_is_in_context() {
    // Given a ToolResult with Success status and Default override.
    let entry = ChatEntry::tool_result("tc-1", "bash", "output", ToolResultStatus::Success);

    // Then it IS in context.
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn failure_tool_result_default_is_in_context() {
    // Given a ToolResult with Failure status and Default override.
    let entry = ChatEntry::tool_result("tc-1", "bash", "error", ToolResultStatus::Failure);

    // Then it IS in context (failed results are still complete).
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn pending_tool_result_forced_include_is_in_context() {
    // Given a ToolResult with Pending status and ForcedInclude override.
    let entry = ChatEntry::tool_result("tc-1", "bash", "", ToolResultStatus::Pending)
        .with_context_override(ContextOverride::ForcedInclude);

    // Then ForcedInclude overrides the pending rule — it IS in context.
    assert!(entry.is_in_context());
}

#[rstest::rstest]
fn pinned_pending_tool_result_default_is_in_context() {
    // Given a ToolResult with Pending status that is pinned.
    let entry = ChatEntry::tool_result("tc-1", "bash", "", ToolResultStatus::Pending)
        .with_pin(PinPosition::Top);

    // Then pin overrides the pending rule — it IS in context.
    assert!(entry.is_in_context());
}

// --- ContextOverride serialization tests ---

#[rstest::rstest]
fn context_override_default_serializes_roundtrip() {
    // Given a default ContextOverride.
    let value = ContextOverride::Default;

    // When serializing and deserializing.
    let json = serde_json::to_string(&value).expect("serialize");
    let back: ContextOverride = serde_json::from_str(&json).expect("deserialize");

    // Then it roundtrips.
    assert_eq!(back, ContextOverride::Default);
}

#[rstest::rstest]
fn context_override_forced_include_serializes_roundtrip() {
    // Given a ForcedInclude ContextOverride.
    let value = ContextOverride::ForcedInclude;

    // When serializing and deserializing.
    let json = serde_json::to_string(&value).expect("serialize");
    let back: ContextOverride = serde_json::from_str(&json).expect("deserialize");

    // Then it roundtrips.
    assert_eq!(back, ContextOverride::ForcedInclude);
    // And the JSON uses snake_case.
    assert_eq!(json, "\"forced_include\"");
}

#[rstest::rstest]
fn context_override_forced_exclude_serializes_roundtrip() {
    // Given a ForcedExclude ContextOverride.
    let value = ContextOverride::ForcedExclude;

    // When serializing and deserializing.
    let json = serde_json::to_string(&value).expect("serialize");
    let back: ContextOverride = serde_json::from_str(&json).expect("deserialize");

    // Then it roundtrips.
    assert_eq!(back, ContextOverride::ForcedExclude);
    // And the JSON uses snake_case.
    assert_eq!(json, "\"forced_exclude\"");
}

#[rstest::rstest]
fn context_override_default_is_default_trait() {
    // Given a default ContextOverride.
    let value = ContextOverride::default();

    // Then it is Default.
    assert_eq!(value, ContextOverride::Default);
}

// --- ChatEntryId::as_uuid ---

#[rstest::rstest]
fn chat_entry_id_as_uuid_returns_inner_uuid() {
    // Given a ChatEntryId.
    let id = ChatEntryId::new();
    let expected = id.to_string();

    // When calling as_uuid.
    let uuid = id.as_uuid();

    // Then it returns the inner UUID (not a leaked default).
    assert_eq!(uuid.to_string(), expected);
}

#[rstest::rstest]
fn chat_entry_id_as_uuid_matches_to_string() {
    // Given a ChatEntryId.
    let id = ChatEntryId::new();

    // When comparing as_uuid output to the string representation.
    let uuid_str = id.as_uuid().to_string();
    let display_str = id.to_string();

    // Then they match.
    assert_eq!(uuid_str, display_str);
}
