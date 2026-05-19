#![allow(clippy::expect_used, clippy::indexing_slicing)]

use ratatui::text::Line;

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
#[case::info(ChatEntry::info(vec![Line::from("info")]))]
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

// --- Info entry tests ---

#[rstest::rstest]
fn info_entry_has_info_kind() {
    // Given text "welcome".
    let lines = vec![Line::from("welcome")];

    // When creating an info entry.
    let entry = ChatEntry::info(lines);

    // Then kind is Info.
    assert!(matches!(entry.kind, ChatEntryKind::Info(_)));
    // And text() returns the plain text.
    assert_eq!(entry.text(), "welcome");
}

#[rstest::rstest]
fn info_kind_str_returns_info() {
    // Given an info entry.
    let entry = ChatEntry::info(vec![Line::from("test")]);
    assert_eq!(entry.kind_str(), "info");
}

#[rstest::rstest]
fn info_text_returns_content() {
    // Given an info entry.
    let entry = ChatEntry::info(vec![Line::from("some hint")]);

    // Then text() returns the content.
    assert_eq!(entry.text(), "some hint");
}

#[rstest::rstest]
fn info_entry_serializes_roundtrip() {
    // Given an info entry.
    let entry = ChatEntry::info(vec![Line::from("Welcome to nullslop!")]);

    // When serializing and deserializing.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then the roundtrip converts Info to System (Info entries are not persisted).
    assert_eq!(
        back.kind,
        ChatEntryKind::System("Welcome to nullslop!".to_owned())
    );
}

#[rstest::rstest]
fn info_entry_pin_position_defaults_to_none() {
    // Given an info entry.
    let entry = ChatEntry::info(vec![Line::from("test")]);

    // Then pin_position is None.
    assert_eq!(entry.pin_position, None);
}

#[rstest::rstest]
fn info_entry_serializes_correct_json_shape() {
    // Given an info entry.
    let entry = ChatEntry::info(vec![Line::from("some info")]);

    // When serializing just the kind.
    let json = serde_json::to_string(&entry.kind).expect("serialize");

    // Then the JSON has the expected shape: {"Info": "..."}.
    let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert!(v.get("Info").is_some(), "should have Info key");
    assert_eq!(v["Info"], "some info");
}

// --- is_pinnable tests ---

#[rstest::rstest]
fn user_entry_is_pinnable() {
    // Given a user entry.
    let entry = ChatEntry::user("hello");

    // Then it is pinnable.
    assert!(entry.is_pinnable());
}

#[rstest::rstest]
fn assistant_entry_is_pinnable() {
    // Given an assistant entry.
    let entry = ChatEntry::assistant("response");

    // Then it is pinnable.
    assert!(entry.is_pinnable());
}

#[rstest::rstest]
fn tool_result_entry_is_pinnable() {
    // Given a tool result entry.
    let entry = ChatEntry::tool_result("id", "bash", "output", ToolResultStatus::Success);

    // Then it is pinnable.
    assert!(entry.is_pinnable());
}

#[rstest::rstest]
fn skill_entry_is_pinnable() {
    // Given a skill entry.
    let entry = ChatEntry::skill("test", "/path", "content");

    // Then it is pinnable.
    assert!(entry.is_pinnable());
}

#[rstest::rstest]
fn info_entry_is_not_pinnable() {
    // Given an info entry.
    let entry = ChatEntry::info(vec![Line::from("welcome")]);

    // Then it is not pinnable.
    assert!(!entry.is_pinnable());
}

#[rstest::rstest]
fn system_entry_is_not_pinnable() {
    // Given a system entry.
    let entry = ChatEntry::system("status");

    // Then it is not pinnable.
    assert!(!entry.is_pinnable());
}

#[rstest::rstest]
fn error_entry_is_not_pinnable() {
    // Given an error entry.
    let entry = ChatEntry::error("error");

    // Then it is not pinnable.
    assert!(!entry.is_pinnable());
}

#[rstest::rstest]
fn actor_entry_is_not_pinnable() {
    // Given an actor entry.
    let entry = ChatEntry::actor("echo", "HELLO");

    // Then it is not pinnable.
    assert!(!entry.is_pinnable());
}

#[rstest::rstest]
fn thinking_entry_is_not_pinnable() {
    // Given a thinking entry.
    let entry = ChatEntry::thinking("reasoning");

    // Then it is not pinnable.
    assert!(!entry.is_pinnable());
}

#[rstest::rstest]
fn tool_call_entry_is_not_pinnable() {
    // Given a tool call entry.
    let entry = ChatEntry::tool_call("id", "bash", "{}");

    // Then it is not pinnable.
    assert!(!entry.is_pinnable());
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
fn pending_tool_result_fingerprint_excludes_content() {
    // Given two pending ToolResult entries with different content.
    let entry1 = ChatEntry::tool_result("id", "bash", "line1", ToolResultStatus::Pending);
    let entry2 = ChatEntry::tool_result("id", "bash", "line1\nline2", ToolResultStatus::Pending);

    // Then their fingerprints are identical (content excluded for pending).
    assert_eq!(entry1.content_fingerprint(), entry2.content_fingerprint());
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
        nullslop_provider::tool_types::TruncationMeta {
            truncated_by: nullslop_provider::tool_types::TruncatedBy::Lines,
            total_lines: 2,
            total_bytes: 19,
            output_lines: 1,
            output_bytes: 9,
        },
    );

    // Then their fingerprints differ (truncation presence affects hash).
    assert_ne!(entry1.content_fingerprint(), entry2.content_fingerprint());
}
