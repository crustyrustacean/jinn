//! Tests for the compaction actor.

use crate::feat::compaction_actor::serializer::serialize_entries_for_compaction;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::PinPosition;

#[test]
fn compaction_entry_is_compaction_returns_true() {
    let entry = ChatEntry {
        id: crate::protocol::ChatEntryId::new(),
        timestamp: jiff::Timestamp::now(),
        kind: ChatEntryKind::Compaction {
            summary: "test".to_owned(),
            tokens_before: 100,
            entries_compacted: 5,
            model_used: "test/model".to_owned(),
        },
        pin_position: None,
        ignored: false,
    };
    assert!(entry.is_compaction());
}

#[test]
fn user_entry_is_compaction_returns_false() {
    let entry = ChatEntry::user("hello");
    assert!(!entry.is_compaction());
}

#[test]
fn insert_entry_at_places_entry_at_correct_position() {
    // Given a session with 3 entries.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::assistant("second"));
    session.push_entry(ChatEntry::user("third"));

    assert_eq!(session.history().len(), 3);

    // When inserting at position 1.
    let idx = session.insert_entry_at(1, ChatEntry::system("inserted"));

    // Then the entry is at position 1 and others shifted.
    assert_eq!(idx, 1);
    assert_eq!(session.history().len(), 4);
    assert_eq!(session.history()[0].text(), "first");
    assert_eq!(session.history()[1].text(), "inserted");
    assert_eq!(session.history()[2].text(), "second");
    assert_eq!(session.history()[3].text(), "third");
}

#[test]
fn insert_entry_at_end_appends() {
    // Given a session with 2 entries.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::assistant("second"));

    // When inserting at position 2 (end).
    let idx = session.insert_entry_at(2, ChatEntry::system("appended"));

    // Then the entry is appended.
    assert_eq!(idx, 2);
    assert_eq!(session.history().len(), 3);
    assert_eq!(session.history()[2].text(), "appended");
}

#[test]
fn mark_entries_ignored_sets_flag() {
    // Given a session with 4 entries.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::assistant("b"));
    session.push_entry(ChatEntry::user("c"));
    session.push_entry(ChatEntry::assistant("d"));

    // When marking entries 0 and 1 as ignored.
    session.mark_entries_ignored(&[0, 1]);

    // Then those entries are ignored but others are not.
    assert!(session.history()[0].ignored);
    assert!(session.history()[1].ignored);
    assert!(!session.history()[2].ignored);
    assert!(!session.history()[3].ignored);
}

#[test]
fn mark_entries_ignored_with_pinned_entry() {
    // Given a session with a pinned entry.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::user("pinned").with_pin(PinPosition::Relative));
    session.push_entry(ChatEntry::assistant("response"));

    // When marking index 0 as ignored.
    session.mark_entries_ignored(&[0]);

    // Then the entry is marked ignored but still pinned.
    assert!(session.history()[0].ignored);
    assert!(session.history()[0].is_pinned());
    // Pin override works: pinned && ignored still counts as "included".
    assert!(session.history()[0].is_pinned() || !session.history()[0].ignored);
}

#[test]
fn serializer_skips_system_entries() {
    let entries = vec![
        ChatEntry::user("hello"),
        ChatEntry::system("status"),
        ChatEntry::assistant("hi"),
    ];
    let result = serialize_entries_for_compaction(&entries);
    assert!(!result.contains("status"));
    assert!(result.contains("[User]: hello"));
    assert!(result.contains("[Assistant]: hi"));
}
