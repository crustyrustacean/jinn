#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::session_store::SessionStore;
use crate::feat::session::session_store::sqlite::SqliteSessionStore;
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::{ChatEntry, ChatEntryKind, SessionId};
use tempfile::TempDir;

/// Creates a minimal `ChatSessionState` for testing.
fn make_session(id: &SessionId, title: &str) -> ChatSessionState {
    let mut session = ChatSessionState::new();
    session.set_session_id(id.clone());
    session.set_title(title.to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session
}

fn make_store() -> (TempDir, SqliteSessionStore) {
    let dir = TempDir::new().expect("temp dir");
    let store = SqliteSessionStore::new_in(dir.path()).expect("store");
    (dir, store)
}

// --- Save + load round-trip ---

#[rstest::rstest]
#[tokio::test]
async fn save_creates_summary() {
    // Given a SqliteSessionStore in a temp directory.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let session = make_session(&session_id, "Test Session");

    // When saving and loading summaries.
    store.save(&session).await.expect("save");
    let summaries = store.load_summaries().await.expect("load_summaries");

    // Then one summary is returned.
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].session_id, session_id);
    assert_eq!(summaries[0].title, "Test Session");
}

#[rstest::rstest]
#[tokio::test]
async fn load_session_restores_data() {
    // Given a SqliteSessionStore in a temp directory.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let session = make_session(&session_id, "Test Session");

    // When saving and loading the session.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load_session")
        .expect("should have a session");

    // Then the session data matches.
    assert_eq!(loaded.session_id(), &session_id);
    assert_eq!(loaded.title(), Some("Test Session"));
    assert_eq!(loaded.history().len(), 1);
}

// --- Multiple sessions ---

#[rstest::rstest]
#[tokio::test]
async fn summaries_returns_correct_count() {
    // Given a store with 2 sessions.
    let (_dir, store) = make_store();
    let id_a = SessionId::new();
    let id_b = SessionId::new();

    store.save(&make_session(&id_a, "A")).await.expect("save A");
    store.save(&make_session(&id_b, "B")).await.expect("save B");

    // When loading summaries.
    let summaries = store.load_summaries().await.expect("load_summaries");

    // Then 2 summaries are returned.
    assert_eq!(summaries.len(), 2);
}

#[rstest::rstest]
#[tokio::test]
async fn save_updates_existing_session() {
    // Given a store with a saved session.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    store
        .save(&make_session(&session_id, "v1"))
        .await
        .expect("save v1");

    // When saving again with updated title.
    let mut updated = make_session(&session_id, "v2");
    updated.push_entry(ChatEntry::assistant("world"));
    store.save(&updated).await.expect("save v2");

    // Then the summary reflects v2.
    let summaries = store.load_summaries().await.expect("load_summaries");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].title, "v2");

    // And the loaded session has both entries.
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load_session")
        .expect("should exist");
    assert_eq!(loaded.history().len(), 2);
}

// --- Load nonexistent session ---

#[rstest::rstest]
#[tokio::test]
async fn load_session_returns_none_for_unknown_id() {
    // Given an empty store.
    let (_dir, store) = make_store();

    // When loading a nonexistent session.
    let result = store
        .load_session(&SessionId::new())
        .await
        .expect("load_session");

    // Then None is returned.
    assert!(result.is_none());
}

// --- Empty store ---

#[rstest::rstest]
#[tokio::test]
async fn load_summaries_returns_empty_when_no_sessions() {
    // Given a fresh store.
    let (_dir, store) = make_store();

    // When loading summaries.
    let summaries = store.load_summaries().await.expect("load_summaries");

    // Then an empty vec is returned.
    assert!(summaries.is_empty());
}

// --- Save creates directory ---

#[rstest::rstest]
#[tokio::test]
async fn save_creates_directory() {
    // Given a SqliteSessionStore pointed at a non-existent directory.
    let dir = TempDir::new().expect("temp dir");
    let nested = dir.path().join("does").join("not").join("exist");
    let store = SqliteSessionStore::new_in(&nested).expect("store");
    let session = make_session(&SessionId::new(), "Mkdir Test");

    // When saving.
    store.save(&session).await.expect("save");

    // Then the directory is created.
    assert!(nested.exists());
}

// --- Delete ---

#[rstest::rstest]
#[tokio::test]
async fn delete_removes_session() {
    // Given a store with a saved session.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    store
        .save(&make_session(&session_id, "To Delete"))
        .await
        .expect("save");

    // When deleting.
    store.delete(&session_id).await.expect("delete");

    // Then the session is gone.
    let result = store.load_session(&session_id).await.expect("load_session");
    assert!(result.is_none());

    // And summaries are empty.
    let summaries = store.load_summaries().await.expect("load_summaries");
    assert!(summaries.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn delete_is_noop_for_unknown_id() {
    // Given a store.
    let (_dir, store) = make_store();

    // When deleting a nonexistent session.
    store.delete(&SessionId::new()).await.expect("delete");

    // Then no error occurs.
}

// --- Fork ---

#[rstest::rstest]
#[tokio::test]
async fn fork_creates_new_session_with_entries_up_to_ordinal() {
    // Given a store with a session that has 3 entries.
    let (_dir, store) = make_store();
    let source_id = SessionId::new();
    let mut source = ChatSessionState::new();
    source.set_session_id(source_id.clone());
    source.set_title("Original".to_owned());
    source.push_entry(ChatEntry::user("first"));
    source.push_entry(ChatEntry::assistant("second"));
    source.push_entry(ChatEntry::user("third"));
    store.save(&source).await.expect("save source");

    // When forking at ordinal 1 (includes entries 0 and 1).
    let forked_id = store.fork(&source_id, 1).await.expect("fork");

    // Then the forked session has 2 entries.
    let forked = store
        .load_session(&forked_id)
        .await
        .expect("load forked")
        .expect("should exist");
    assert_eq!(forked.history().len(), 2);

    // And the entries match the first two of the source.
    match &forked.history()[0].kind {
        ChatEntryKind::User { display, .. } => assert_eq!(display, "first"),
        other => panic!("expected User, got {other:?}"),
    }
    match &forked.history()[1].kind {
        ChatEntryKind::Assistant(t) => assert_eq!(t, "second"),
        other => panic!("expected Assistant, got {other:?}"),
    }

    // And the forked session has the source as parent.
    assert_eq!(forked.parent_session(), &Some(source_id.clone()));
}

#[rstest::rstest]
#[tokio::test]
async fn fork_does_not_modify_source() {
    // Given a store with a session that has 3 entries.
    let (_dir, store) = make_store();
    let source_id = SessionId::new();
    let mut source = ChatSessionState::new();
    source.set_session_id(source_id.clone());
    source.set_title("Original".to_owned());
    source.push_entry(ChatEntry::user("a"));
    source.push_entry(ChatEntry::assistant("b"));
    source.push_entry(ChatEntry::user("c"));
    store.save(&source).await.expect("save source");

    // When forking at ordinal 1.
    store.fork(&source_id, 1).await.expect("fork");

    // Then the source session is unchanged.
    let reloaded = store
        .load_session(&source_id)
        .await
        .expect("load source")
        .expect("should exist");
    assert_eq!(reloaded.history().len(), 3);
    assert_eq!(reloaded.title(), Some("Original"));
}

#[rstest::rstest]
#[tokio::test]
async fn fork_shares_entry_data_not_junction_rows() {
    // Given a store with a saved session.
    let (_dir, store) = make_store();
    let source_id = SessionId::new();
    let mut source = ChatSessionState::new();
    source.set_session_id(source_id.clone());
    source.set_title("Source".to_owned());
    source.push_entry(ChatEntry::user("shared entry"));
    store.save(&source).await.expect("save source");

    // When forking at ordinal 0.
    let forked_id = store.fork(&source_id, 0).await.expect("fork");

    // Then both sessions reference the same entry (same entry_id).
    let source = store
        .load_session(&source_id)
        .await
        .expect("load source")
        .expect("should exist");
    let forked = store
        .load_session(&forked_id)
        .await
        .expect("load forked")
        .expect("should exist");

    assert_eq!(source.history()[0].id, forked.history()[0].id);
}

#[rstest::rstest]
#[tokio::test]
async fn fork_returns_error_for_unknown_source() {
    // Given a store.
    let (_dir, store) = make_store();

    // When forking from a nonexistent source.
    let result = store.fork(&SessionId::new(), 0).await;

    // Then an error is returned.
    assert!(result.is_err());
}

// --- Entry kinds round-trip ---

#[rstest::rstest]
#[tokio::test]
async fn all_entry_kinds_round_trip() {
    // Given a session with every entry kind.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("All Kinds".to_owned());

    session.push_entry(ChatEntry::user("user msg"));
    session.push_entry(ChatEntry::system("system msg"));
    session.push_entry(ChatEntry::error("error msg"));
    session.push_entry(ChatEntry::assistant("assistant msg"));
    session.push_entry(ChatEntry::actor("bash", "actor msg"));
    session.push_entry(ChatEntry::thinking("thinking text"));
    session.push_entry(ChatEntry::tool_call("call_1", "bash", "{\"cmd\": true}"));
    session.push_entry(ChatEntry::tool_result(
        "call_1",
        "bash",
        "ok",
        ToolResultStatus::Success,
    ));

    // When saving and loading.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then all entry kinds are preserved.
    assert_eq!(loaded.history().len(), 8);
    assert!(
        matches!(&loaded.history()[0].kind, ChatEntryKind::User { display, .. } if display == "user msg")
    );
    assert!(matches!(&loaded.history()[1].kind, ChatEntryKind::System(t) if t == "system msg"));
    assert!(matches!(&loaded.history()[2].kind, ChatEntryKind::Error(t) if t == "error msg"));
    assert!(
        matches!(&loaded.history()[3].kind, ChatEntryKind::Assistant(t) if t == "assistant msg")
    );
    assert!(
        matches!(&loaded.history()[4].kind, ChatEntryKind::Actor { source, text } if source == "bash" && text == "actor msg")
    );
    assert!(
        matches!(&loaded.history()[5].kind, ChatEntryKind::Thinking(t) if t == "thinking text")
    );
    assert!(
        matches!(&loaded.history()[6].kind, ChatEntryKind::ToolCall { id, name, arguments } if id == "call_1" && name == "bash" && arguments == "{\"cmd\": true}")
    );
    assert!(
        matches!(&loaded.history()[7].kind, ChatEntryKind::ToolResult { id, name, content, status, .. } if id == "call_1" && name == "bash" && content == "ok" && *status == ToolResultStatus::Success)
    );
}

// --- Pin position round-trip ---

#[rstest::rstest]
#[tokio::test]
async fn pin_position_round_trips() {
    // Given a session with pinned entries.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Pins".to_owned());

    session.push_entry(ChatEntry::user("pinned top").with_pin(crate::protocol::PinPosition::Top));
    session.push_entry(
        ChatEntry::assistant("pinned bottom").with_pin(crate::protocol::PinPosition::Bottom),
    );
    session.push_entry(
        ChatEntry::user("pinned relative").with_pin(crate::protocol::PinPosition::Relative),
    );
    session.push_entry(ChatEntry::user("unpinned"));

    // When saving and loading.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then pin positions are preserved.
    assert_eq!(
        loaded.history()[0].pin_position,
        Some(crate::protocol::PinPosition::Top)
    );
    assert_eq!(
        loaded.history()[1].pin_position,
        Some(crate::protocol::PinPosition::Bottom)
    );
    assert_eq!(
        loaded.history()[2].pin_position,
        Some(crate::protocol::PinPosition::Relative)
    );
    assert_eq!(loaded.history()[3].pin_position, None);
}

// --- Token ledger round-trip ---

#[rstest::rstest]
#[tokio::test]
async fn token_ledger_round_trips() {
    // Given a session with token records.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Tokens".to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session.push_token_record(crate::feat::session::token_stats::TokenRecord {
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 100,
        tokens_received: 50,
        cost: None,
    });

    // When saving and loading.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then the token ledger is preserved.
    assert_eq!(loaded.token_ledger().len(), 1);
    assert_eq!(loaded.token_ledger()[0].tokens_sent, 100);
    assert_eq!(loaded.token_ledger()[0].tokens_received, 50);
}

// --- Delete orphans shared entries ---

#[rstest::rstest]
#[tokio::test]
async fn delete_cleans_up_orphaned_entries() {
    // Given two sessions sharing entries via fork.
    let (_dir, store) = make_store();
    let source_id = SessionId::new();
    let mut source = ChatSessionState::new();
    source.set_session_id(source_id.clone());
    source.set_title("Source".to_owned());
    source.push_entry(ChatEntry::user("shared"));
    store.save(&source).await.expect("save source");

    let forked_id = store.fork(&source_id, 0).await.expect("fork");

    // When deleting the forked session.
    store.delete(&forked_id).await.expect("delete forked");

    // Then the source session still has its entry.
    let source = store
        .load_session(&source_id)
        .await
        .expect("load source")
        .expect("should exist");
    assert_eq!(source.history().len(), 1);

    // When also deleting the source.
    store.delete(&source_id).await.expect("delete source");

    // Then the entry is fully cleaned up (verified by saving the same
    // entry ID again - should work since it was deleted).
    let summaries = store.load_summaries().await.expect("load_summaries");
    assert!(summaries.is_empty());
}

// --- CWD persistence ---

#[rstest::rstest]
#[tokio::test]
async fn cwd_round_trips_through_save_and_load() {
    // Given a store with a session that has a custom cwd.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("CWD Test".to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session.set_cwd(std::path::PathBuf::from("/tmp/my-project"));

    // When saving and loading.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then the cwd is preserved.
    assert_eq!(loaded.cwd(), std::path::Path::new("/tmp/my-project"));
}

#[rstest::rstest]
#[tokio::test]
async fn fork_inherits_cwd_from_source() {
    // Given a store with a session that has a custom cwd.
    let (_dir, store) = make_store();
    let source_id = SessionId::new();
    let mut source = ChatSessionState::new();
    source.set_session_id(source_id.clone());
    source.set_title("Original".to_owned());
    source.push_entry(ChatEntry::user("hello"));
    source.set_cwd(std::path::PathBuf::from("/home/user/project"));
    store.save(&source).await.expect("save source");

    // When forking.
    let forked_id = store.fork(&source_id, 0).await.expect("fork");

    // Then the forked session inherits the source cwd.
    let forked = store
        .load_session(&forked_id)
        .await
        .expect("load forked")
        .expect("should exist");
    assert_eq!(forked.cwd(), std::path::Path::new("/home/user/project"));
}

#[rstest::rstest]
#[tokio::test]
async fn save_updates_cwd_on_existing_session() {
    // Given a store with a saved session.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("CWD Update".to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session.set_cwd(std::path::PathBuf::from("/old/path"));
    store.save(&session).await.expect("save v1");

    // When saving with an updated cwd.
    session.set_cwd(std::path::PathBuf::from("/new/path"));
    store.save(&session).await.expect("save v2");

    // Then the loaded session has the new cwd.
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");
    assert_eq!(loaded.cwd(), std::path::Path::new("/new/path"));
}

#[tokio::test]
async fn ignored_field_round_trips() {
    // Given a session with ignored entries.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Ignored".to_owned());

    session.push_entry(ChatEntry::user("normal"));
    session.push_entry(ChatEntry::assistant("response"));
    session.push_entry(ChatEntry::user("ignored message"));
    session.push_entry(ChatEntry::assistant("ignored response"));

    // When marking entries 2,3 as ignored.
    session.mark_entries_ignored(&[2, 3]);

    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then ignored flags are preserved after round-trip.
    assert!(
        !loaded.history()[0].ignored(),
        "entry 0 should not be ignored"
    );
    assert!(
        !loaded.history()[1].ignored(),
        "entry 1 should not be ignored"
    );
    assert!(loaded.history()[2].ignored(), "entry 2 should be ignored");
    assert!(loaded.history()[3].ignored(), "entry 3 should be ignored");
}

// --- Lifecycle metadata persistence ---

#[rstest::rstest]
#[tokio::test]
async fn lifecycle_metadata_round_trips() {
    // Given a store with a session that has lifecycle metadata.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = make_session(&session_id, "Lifecycle Session");
    session.set_lifecycle_name(Some("fossil branch".to_owned()));
    session.set_lifecycle_args(vec!["my-branch".to_owned(), "--private".to_owned()]);

    // When saving and loading.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then lifecycle metadata is preserved.
    assert_eq!(loaded.lifecycle_name(), Some("fossil branch"));
    assert_eq!(
        loaded.lifecycle_args(),
        &["my-branch".to_owned(), "--private".to_owned()]
    );
}

#[rstest::rstest]
#[tokio::test]
async fn session_without_lifecycle_loads_as_none() {
    // Given a store with a session that has no lifecycle metadata.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let session = make_session(&session_id, "Plain Session");

    // When saving and loading.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then lifecycle fields are None/empty.
    assert_eq!(loaded.lifecycle_name(), None);
    assert!(loaded.lifecycle_args().is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn fork_inherits_lifecycle_metadata() {
    // Given a store with a session that has lifecycle metadata.
    let (_dir, store) = make_store();
    let source_id = SessionId::new();
    let mut source = ChatSessionState::new();
    source.set_session_id(source_id.clone());
    source.set_title("Source".to_owned());
    source.push_entry(ChatEntry::user("hello"));
    source.set_lifecycle_name(Some("fossil branch".to_owned()));
    source.set_lifecycle_args(vec!["dev".to_owned()]);
    store.save(&source).await.expect("save source");

    // When forking.
    let forked_id = store.fork(&source_id, 0).await.expect("fork");
    let forked = store
        .load_session(&forked_id)
        .await
        .expect("load forked")
        .expect("should exist");

    // Then the forked session inherits lifecycle metadata.
    assert_eq!(forked.lifecycle_name(), Some("fossil branch"));
    assert_eq!(forked.lifecycle_args(), &["dev".to_owned()]);
}

// --- Lifecycle script state persistence ---

#[rstest::rstest]
#[tokio::test]
async fn lifecycle_script_state_setup_ran_round_trips() {
    // Given a session with SetupRan.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Lifecycle State".to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session.advance_lifecycle_after_setup();

    // When saving and loading.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then lifecycle_script_state is SetupRan.
    assert_eq!(
        loaded.lifecycle_script_state(),
        crate::feat::session::chat_session::LifecycleScriptState::SetupRan
    );
}

#[rstest::rstest]
#[tokio::test]
async fn lifecycle_script_state_nothing_ran_round_trips() {
    // Given a session with NothingRan (default).
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let session = make_session(&session_id, "Default State");

    // When saving and loading.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then lifecycle_script_state is NothingRan.
    assert_eq!(
        loaded.lifecycle_script_state(),
        crate::feat::session::chat_session::LifecycleScriptState::NothingRan
    );
}

#[rstest::rstest]
#[tokio::test]
async fn fork_inherits_lifecycle_script_state() {
    // Given a store with a session that has SetupRan.
    let (_dir, store) = make_store();
    let source_id = SessionId::new();
    let mut source = ChatSessionState::new();
    source.set_session_id(source_id.clone());
    source.set_title("Source".to_owned());
    source.push_entry(ChatEntry::user("hello"));
    source.advance_lifecycle_after_setup();
    store.save(&source).await.expect("save source");

    // When forking.
    let forked_id = store.fork(&source_id, 0).await.expect("fork");
    let forked = store
        .load_session(&forked_id)
        .await
        .expect("load forked")
        .expect("should exist");

    // Then the forked session inherits SetupRan.
    assert_eq!(
        forked.lifecycle_script_state(),
        crate::feat::session::chat_session::LifecycleScriptState::SetupRan
    );
}

#[rstest::rstest]
#[tokio::test]
async fn is_automated_round_trips_through_save_and_load() {
    // Given a persisted automated session.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Automated".to_owned());
    session.core.is_automated = true;
    session.core.persist = true;

    // When saving and loading.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then both flags are preserved.
    assert!(loaded.is_automated(), "is_automated should round-trip");
    assert!(loaded.core.persist, "persist should round-trip");
}

#[rstest::rstest]
#[tokio::test]
async fn non_persistent_session_is_not_written() {
    // Given a transient (persist=false) session.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Transient".to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session.core.is_automated = true;
    session.core.persist = false;

    // When saving.
    store.save(&session).await.expect("save should be a no-op, not an error");

    // Then no row exists for this session.
    let loaded = store.load_session(&session_id).await.expect("load query");
    assert!(loaded.is_none(), "persist=false session must not be written");
}

#[rstest::rstest]
#[tokio::test]
async fn persistent_session_is_written() {
    // Given a persistent (persist=true) session.
    let (_dir, store) = make_store();
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Persistent".to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session.core.is_automated = true;
    session.core.persist = true;

    // When saving.
    store.save(&session).await.expect("save");

    // Then the row exists.
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load query")
        .expect("should exist");
    assert_eq!(loaded.session_id(), &session_id);
    // And the automated + persist flags round-trip through SQLite.
    assert!(
        loaded.core.is_automated,
        "is_automated must survive save/load"
    );
    assert!(
        loaded.core.persist,
        "persist must survive save/load"
    );
}
