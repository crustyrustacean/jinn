use super::*;

#[test]
fn chat_entry_id_is_unique() {
    // Given two generated IDs.
    let id1 = ChatEntryId::new();
    let id2 = ChatEntryId::new();

    // Then they are not equal.
    assert_ne!(id1, id2);
}

#[test]
fn chat_entry_id_is_valid_uuid() {
    // Given a generated ID.
    let id = ChatEntryId::new();

    // Then the string representation is a valid UUID.
    let s = id.to_string();
    assert!(uuid::Uuid::parse_str(&s).is_ok());
}

#[test]
fn chat_entry_id_serialization_roundtrip() {
    // Given a ChatEntryId.
    let id = ChatEntryId::new();

    // When serialized and deserialized.
    let json = serde_json::to_string(&id).expect("serialize");
    let back: ChatEntryId = serde_json::from_str(&json).expect("deserialize");

    // Then it matches the original.
    assert_eq!(back, id);
}

#[test]
fn user_entry_has_user_kind() {
    // Given text "hello".
    let text = "hello";

    // When creating a user entry.
    let entry = ChatEntry::user(text);

    // Then kind is User("hello").
    assert_eq!(entry.kind, ChatEntryKind::User("hello".to_owned()));
}

#[test]
fn system_entry_has_system_kind() {
    // Given text "ready".
    let text = "ready";

    // When creating a system entry.
    let entry = ChatEntry::system(text);

    // Then kind is System("ready").
    assert_eq!(entry.kind, ChatEntryKind::System("ready".to_owned()));
}

#[test]
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

#[test]
fn chat_entry_serialization_roundtrip() {
    // Given a ChatEntry.
    let entry = ChatEntry::user("hello");

    // When serialized and deserialized.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then it matches the original.
    assert_eq!(back.id, entry.id);
    assert_eq!(back.kind, entry.kind);
    assert_eq!(back.timestamp, entry.timestamp);
}

#[test]
fn assistant_entry_has_assistant_kind() {
    let text = "hello";
    let entry = ChatEntry::assistant(text);
    assert_eq!(entry.kind, ChatEntryKind::Assistant("hello".to_owned()));
}

#[test]
fn assistant_entry_serialization_roundtrip() {
    let entry = ChatEntry::assistant("hello");
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.id, entry.id);
    assert_eq!(back.kind, entry.kind);
    assert_eq!(back.timestamp, entry.timestamp);
}

#[test]
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

#[test]
fn actor_entry_serialization_roundtrip() {
    // Given an actor ChatEntry.
    let entry = ChatEntry::actor("echo", "hello");

    // When serialized and deserialized.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then it matches the original.
    assert_eq!(back.id, entry.id);
    assert_eq!(back.kind, entry.kind);
    assert_eq!(back.timestamp, entry.timestamp);
}

#[test]
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

#[test]
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

#[test]
fn tool_call_entry_serialization_roundtrip() {
    // Given a tool call ChatEntry.
    let entry = ChatEntry::tool_call("call_1", "echo", r#"{"input":"hi"}"#);

    // When serialized and deserialized.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then it matches the original.
    assert_eq!(back.id, entry.id);
    assert_eq!(back.kind, entry.kind);
    assert_eq!(back.timestamp, entry.timestamp);
}

#[test]
fn tool_result_entry_serialization_roundtrip() {
    // Given a tool result ChatEntry.
    let entry = ChatEntry::tool_result("call_1", "echo", "hi", true);

    // When serialized and deserialized.
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: ChatEntry = serde_json::from_str(&json).expect("deserialize");

    // Then it matches the original.
    assert_eq!(back.id, entry.id);
    assert_eq!(back.kind, entry.kind);
    assert_eq!(back.timestamp, entry.timestamp);
}
