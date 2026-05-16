use super::chat_entry::*;

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
    let entry = ChatEntry::tool_result(id, name, content, true);

    // Then kind is ToolResult with correct fields.
    assert_eq!(
        entry.kind,
        ChatEntryKind::ToolResult {
            id: "call_123".to_owned(),
            name: "echo".to_owned(),
            content: "hi".to_owned(),
            success: true,
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
#[case::tool_result(ChatEntry::tool_result("id", "name", "content", true))]
#[case::skill(ChatEntry::skill("name", "/path", "content"))]
#[case::info(ChatEntry::info("info"))]
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
    let text = "welcome";

    // When creating an info entry.
    let entry = ChatEntry::info(text);

    // Then kind is Info("welcome").
    assert_eq!(entry.kind, ChatEntryKind::Info("welcome".to_owned()));
}

#[rstest::rstest]
fn info_kind_str_returns_info() {
    // Given an info entry.
    let entry = ChatEntry::info("test");

    // Then kind_str returns "info".
    assert_eq!(entry.kind_str(), "info");
}

#[rstest::rstest]
fn info_text_returns_content() {
    // Given an info entry.
    let entry = ChatEntry::info("some hint");

    // Then text() returns the content.
    assert_eq!(entry.text(), "some hint");
}

#[rstest::rstest]
fn info_entry_serializes_roundtrip() {
    // Given an info entry.
    let entry = ChatEntry::info("Welcome to nullslop!");

    // When serializing and deserializing.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then the roundtrip preserves the kind.
    assert_eq!(
        back.kind,
        ChatEntryKind::Info("Welcome to nullslop!".to_owned())
    );
}

#[rstest::rstest]
fn info_entry_pin_position_defaults_to_none() {
    // Given an info entry.
    let entry = ChatEntry::info("test");

    // Then pin_position is None.
    assert_eq!(entry.pin_position, None);
}

#[rstest::rstest]
fn info_entry_serializes_correct_json_shape() {
    // Given an info entry.
    let entry = ChatEntry::info("some info");

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
    let entry = ChatEntry::tool_result("id", "bash", "output", true);

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
    let entry = ChatEntry::info("welcome");

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
