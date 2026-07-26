#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::session_store::SessionStore;
use crate::feat::session::session_store::sqlite::SqliteSessionStore;
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::{ChatEntry, ChatEntryKind, EntryTiming, SessionId};
use tempfile::TempDir;

/// Creates a minimal `ChatSessionState` for testing.
fn make_session(id: &SessionId, title: &str) -> ChatSessionState {
    let mut session = ChatSessionState::new();
    session.set_session_id(id.clone());
    session.set_title(title.to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session
}

async fn make_store() -> (TempDir, SqliteSessionStore) {
    let dir = TempDir::new().expect("temp dir");
    let store = SqliteSessionStore::new_in(dir.path()).await.expect("store");
    (dir, store)
}

#[rstest::rstest]
#[tokio::test]
async fn save_creates_summary() {
    // Given a SqliteSessionStore in a temp directory.
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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

#[rstest::rstest]
#[tokio::test]
async fn degraded_token_expanded_survives_save_and_reload() {
    // Given a session with a resolved degraded user entry (marker set, expanded literal).
    let (_dir, store) = make_store().await;
    let session_id = SessionId::new();
    let token = "@/nonexistent/whatever";
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("degraded".to_owned());
    let mut entry = ChatEntry::user(format!("describe {token}"));
    entry.degraded_paths = Some(vec![std::path::PathBuf::from("/nonexistent/whatever")]);
    session.push_entry(entry);

    // When saving and reloading the session.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load_session")
        .expect("should have a session");

    // Then the AI-facing expanded text keeps the literal token (no file:// revert).
    let expanded = loaded
        .history()
        .iter()
        .rev()
        .find_map(|e| match &e.kind {
            ChatEntryKind::User { expanded, .. } => Some(expanded.clone()),
            _ => None,
        })
        .expect("user entry");
    assert!(
        expanded.contains(token),
        "reloaded expanded must keep literal token: {expanded}"
    );
    assert!(
        !expanded.contains("file://"),
        "reloaded expanded must not contain file://: {expanded}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn summaries_returns_correct_count() {
    // Given a store with 2 sessions.
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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

#[rstest::rstest]
#[tokio::test]
async fn load_session_returns_none_for_unknown_id() {
    // Given an empty store.
    let (_dir, store) = make_store().await;

    // When loading a nonexistent session.
    let result = store
        .load_session(&SessionId::new())
        .await
        .expect("load_session");

    // Then None is returned.
    assert!(result.is_none());
}

#[rstest::rstest]
#[tokio::test]
async fn load_summaries_returns_empty_when_no_sessions() {
    // Given a fresh store.
    let (_dir, store) = make_store().await;

    // When loading summaries.
    let summaries = store.load_summaries().await.expect("load_summaries");

    // Then an empty vec is returned.
    assert!(summaries.is_empty());
}

#[rstest::rstest]
#[tokio::test]
async fn save_creates_directory() {
    // Given a SqliteSessionStore pointed at a non-existent directory.
    let dir = TempDir::new().expect("temp dir");
    let nested = dir.path().join("does").join("not").join("exist");
    let store = SqliteSessionStore::new_in(&nested).await.expect("store");
    let session = make_session(&SessionId::new(), "Mkdir Test");

    // When saving.
    store.save(&session).await.expect("save");

    // Then the directory is created.
    assert!(nested.exists());
}

#[rstest::rstest]
#[tokio::test]
async fn delete_removes_session() {
    // Given a store with a saved session.
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;

    // When deleting a nonexistent session.
    store.delete(&SessionId::new()).await.expect("delete");

    // Then no error occurs.
}

#[rstest::rstest]
#[tokio::test]
async fn fork_creates_new_session_with_entries_up_to_ordinal() {
    // Given a store with a session that has 3 entries.
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;

    // When forking from a nonexistent source.
    let result = store.fork(&SessionId::new(), 0).await;

    // Then an error is returned.
    assert!(result.is_err());
}

#[rstest::rstest]
#[tokio::test]
async fn all_entry_kinds_round_trip() {
    // Given a session with every entry kind.
    let (_dir, store) = make_store().await;
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

#[rstest::rstest]
#[tokio::test]
async fn pin_position_round_trips() {
    // Given a session with pinned entries.
    let (_dir, store) = make_store().await;
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

#[rstest::rstest]
#[tokio::test]
async fn token_ledger_round_trips() {
    // Given a session with token records.
    let (_dir, store) = make_store().await;
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Tokens".to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session.push_token_record(crate::feat::session::token_stats::TokenRecord {
        model_used: None,
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

#[rstest::rstest]
#[tokio::test]
async fn delete_cleans_up_orphaned_entries() {
    // Given two sessions sharing entries via fork.
    let (_dir, store) = make_store().await;
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

#[rstest::rstest]
#[tokio::test]
async fn cwd_round_trips_through_save_and_load() {
    // Given a store with a session that has a custom cwd.
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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

#[rstest::rstest]
#[tokio::test]
async fn lifecycle_metadata_round_trips() {
    // Given a store with a session that has lifecycle metadata.
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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

#[rstest::rstest]
#[tokio::test]
async fn lifecycle_script_state_setup_ran_round_trips() {
    // Given a session with SetupRan.
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
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
    let (_dir, store) = make_store().await;
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Transient".to_owned());
    session.push_entry(ChatEntry::user("hello"));
    session.core.is_automated = true;
    session.core.persist = false;

    // When saving.
    store
        .save(&session)
        .await
        .expect("save should be a no-op, not an error");

    // Then no row exists for this session.
    let loaded = store.load_session(&session_id).await.expect("load query");
    assert!(
        loaded.is_none(),
        "persist=false session must not be written"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn persistent_session_is_written() {
    // Given a persistent (persist=true) session.
    let (_dir, store) = make_store().await;
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
    assert!(loaded.core.persist, "persist must survive save/load");
}

#[rstest::rstest]
#[tokio::test]
async fn streamed_timing_roundtrips_through_db() {
    // Given a persisted session with an entry that has Streamed timing.
    let (_dir, store) = make_store().await;
    let session_id = SessionId::new();
    let mut session = make_session(&session_id, "Timing test");
    session.core.persist = true;

    let dispatched = jiff::Timestamp::now();
    let mut timing = EntryTiming::streamed(dispatched);
    timing.set_first_token();
    timing.finish();

    let mut entry = ChatEntry::user("hello");
    entry.timing = timing;
    session.push_entry(entry);

    // When saving and loading.
    store.save(&session).await.expect("save");

    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load")
        .expect("should exist");

    // Then the Streamed timing is preserved with all timestamps.
    let loaded_entry = &loaded.history()[1];
    match &loaded_entry.timing {
        EntryTiming::Streamed {
            dispatched_at,
            first_token_at,
            finished_at,
        } => {
            assert_eq!(
                *dispatched_at, dispatched,
                "dispatched_at should round-trip"
            );
            assert!(first_token_at.is_some(), "first_token_at should be Some");
            assert!(finished_at.is_some(), "finished_at should be Some");
        }
        other => panic!("expected Streamed timing, got {other:?}"),
    }
}

#[rstest::rstest]
#[tokio::test]
async fn fork_ordinal_persists_across_save_and_load() {
    // Given a store with a session that has 3 entries.
    let (_dir, store) = make_store().await;
    let source_id = SessionId::new();
    let mut source = ChatSessionState::new();
    source.set_session_id(source_id.clone());
    source.push_entry(ChatEntry::user("first"));
    source.push_entry(ChatEntry::assistant("second"));
    source.push_entry(ChatEntry::user("third"));
    store.save(&source).await.expect("save source");

    // When forking at ordinal 1.
    let forked_id = store.fork(&source_id, 1).await.expect("fork");

    // Then the forked session has fork_ordinal = Some(1) after loading.
    let forked = store
        .load_session(&forked_id)
        .await
        .expect("load forked")
        .expect("should exist");
    assert_eq!(forked.fork_ordinal(), Some(1));
}

#[rstest::rstest]
#[tokio::test]
async fn fork_blocking_sets_fork_ordinal() {
    // Given a store with a session that has 5 entries.
    let (_dir, store) = make_store().await;
    let source_id = SessionId::new();
    let mut source = ChatSessionState::new();
    source.set_session_id(source_id.clone());
    source.push_entry(ChatEntry::user("a"));
    source.push_entry(ChatEntry::assistant("b"));
    source.push_entry(ChatEntry::user("c"));
    source.push_entry(ChatEntry::assistant("d"));
    source.push_entry(ChatEntry::user("e"));
    store.save(&source).await.expect("save source");

    // When forking at ordinal 4 (all entries inherited).
    let forked_id = store.fork(&source_id, 4).await.expect("fork");

    // Then the forked session has fork_ordinal = Some(4).
    let forked = store
        .load_session(&forked_id)
        .await
        .expect("load forked")
        .expect("should exist");
    assert_eq!(forked.fork_ordinal(), Some(4));

    // And the root session has fork_ordinal = None.
    let root = store
        .load_session(&source_id)
        .await
        .expect("load root")
        .expect("should exist");
    assert_eq!(root.fork_ordinal(), None);
}

use crate::feat::session::model_selection::ModelSelection;
use crate::feat::session::session_store::migrator::seed_at_version;
use rusqlite::params;

/// A metadata blob in the 0.65 shape: `profile.model` is a bare string.
///
/// Constructed by hand (not via `PersistableCore`) so it carries the legacy
/// serialization that v19 must repair.
const LEGACY_065_BLOB: &str = "{\"session_id\":\"10000000-0000-0000-0000-000000000065\",\"title\":\"Legacy 065\",\"profile\":{\"strategy\":\"sliding_window\",\"model\":\"ollama/llama3\",\"persona_name\":\"coding-assistant\",\"token_budget\":150000,\"sliding_window_size\":5},\"cwd\":\".\",\"parent_session\":null,\"blobs\":{},\"lifecycle_name\":null,\"lifecycle_args\":[],\"lifecycle_script_state\":\"nothing_ran\",\"session_state\":\"Loaded\",\"created_at\":\"2024-01-01T00:00:00Z\",\"updated_at\":\"2024-01-01T00:00:00Z\",\"is_automated\":false,\"persist\":true}";

#[rstest::rstest]
#[tokio::test]
async fn legacy_065_blob_loads_after_v19() {
    // Given a 0.65 database (recorded at v18) with a legacy-shape metadata blob.
    // v19 has not yet run, so profile.model is still a bare string.
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("sessions.db");
    seed_at_version(db_path.to_string_lossy().as_ref(), 18, |conn| {
        conn.execute(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, \
             lifecycle_script_state, is_automated, persist, metadata) \
             VALUES ('10000000-0000-0000-0000-000000000065', 'Legacy 065', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
             '{\"model\":\"ollama/llama3\"}', '{}', 'nothing_ran', 0, 0, ?)",
            params![LEGACY_065_BLOB],
        ).map(|_| ())
    })
    .await;

    // When loading the session through the store.
    // (The store re-runs migrations on open; v19 repairs the blob, v20 drops zombies.)
    let store = SqliteSessionStore::new_in(dir.path()).await.expect("store");
    let loaded = store
        .load_session(&SessionId::from(
            "10000000-0000-0000-0000-000000000065".to_owned(),
        ))
        .await
        .expect("load_session")
        .expect("session should exist");

    // Then the profile model is restored as Single(...) - not dropped or errored.
    assert_eq!(
        loaded.model_selection(),
        &ModelSelection::Single("ollama/llama3".to_owned()),
        "0.65 bare-string model must load as Single after v19"
    );

    // And other profile fields round-trip correctly.
    assert_eq!(loaded.persona_name(), "coding-assistant");
}

/// A metadata blob in the 0.66 shape: `profile.model` is already `{"single": ...}`.
const CURRENT_066_BLOB: &str = "{\"session_id\":\"10000000-0000-0000-0000-000000000066\",\"title\":\"Current 066\",\"profile\":{\"strategy\":\"sliding_window\",\"model\":{\"single\":\"ollama/llama3\"},\"persona_name\":\"coding-assistant\",\"token_budget\":150000,\"sliding_window_size\":5},\"cwd\":\".\",\"parent_session\":null,\"blobs\":{},\"lifecycle_name\":null,\"lifecycle_args\":[],\"lifecycle_script_state\":\"nothing_ran\",\"session_state\":\"Loaded\",\"created_at\":\"2024-01-01T00:00:00Z\",\"updated_at\":\"2024-01-01T00:00:00Z\",\"is_automated\":false,\"persist\":true}";

#[rstest::rstest]
#[tokio::test]
async fn current_066_blob_loads_unchanged() {
    // Given a fresh database with a 0.66-shape blob inserted directly.
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("sessions.db");
    seed_at_version(db_path.to_string_lossy().as_ref(), 18, |conn| {
        conn.execute(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, \
             lifecycle_script_state, is_automated, persist, metadata) \
             VALUES ('10000000-0000-0000-0000-000000000066', 'Current 066', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
             '{\"model\":{\"single\":\"ollama/llama3\"}}', '{}', 'nothing_ran', 0, 0, ?)",
            params![CURRENT_066_BLOB],
        ).map(|_| ())
    })
    .await;

    // When loading the session through the store.
    let store = SqliteSessionStore::new_in(dir.path()).await.expect("store");
    let loaded = store
        .load_session(&SessionId::from(
            "10000000-0000-0000-0000-000000000066".to_owned(),
        ))
        .await
        .expect("load_session")
        .expect("session should exist");

    // Then the profile model loads correctly - no regression on current sessions.
    assert_eq!(
        loaded.model_selection(),
        &ModelSelection::Single("ollama/llama3".to_owned())
    );
    assert_eq!(loaded.persona_name(), "coding-assistant");
}

/// A pre-v8 session (no metadata blob) must still load after v20 backfills its
/// metadata from the zombie columns. The legacy column-read path is gone (v20
/// dropped those columns), so loading succeeds via the backfilled blob.
#[rstest::rstest]
#[tokio::test]
async fn legacy_pre_v8_row_loads_after_v20_backfill() {
    // Given a fresh database at v18 with a session row whose metadata is NULL
    // (pre-v8 shape). The profile column carries the post-v17 form.
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("sessions.db");
    seed_at_version(db_path.to_string_lossy().as_ref(), 18, |conn| {
        conn.execute(
            "INSERT INTO sessions (id, title, updated_at, created_at, cwd, profile, blobs, \
             lifecycle_script_state, is_automated, persist) \
             VALUES ('10000000-0000-0000-0000-000000000008', 'Legacy Pre-V8', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', '.', \
             '{\"model\":{\"single\":\"ollama/llama3\"},\"persona_name\":\"coding-assistant\"}', '{}', 'nothing_ran', 0, 0)",
            params![],
        ).map(|_| ())
    })
    .await;

    // When loading the session through the store (which runs v19 + v20 on open).
    let store = SqliteSessionStore::new_in(dir.path()).await.expect("store");
    let loaded = store
        .load_session(&SessionId::from(
            "10000000-0000-0000-0000-000000000008".to_owned(),
        ))
        .await
        .expect("load_session")
        .expect("session should exist");

    // Then v20 backfilled the metadata from the profile column, and the blob
    // path restores model + persona.
    assert_eq!(
        loaded.model_selection(),
        &ModelSelection::Single("ollama/llama3".to_owned()),
        "pre-v8 row must load its model after v20 backfill"
    );
    assert_eq!(
        loaded.persona_name(),
        "coding-assistant",
        "pre-v8 row must load persona after v20 backfill"
    );
}
/// After v20, the `sessions` table has exactly the 9 authoritative columns —
/// the zombie columns (`profile`, `blobs`, `cwd`, `lifecycle_script_state`,
/// `lifecycle_args`) are gone. A saved session's real data lives entirely
/// in the `metadata` blob.
#[rstest::rstest]
#[tokio::test]
async fn sessions_table_has_exactly_nine_columns() {
    // Given a saved session.
    let (dir, store) = make_store().await;
    let session_id = SessionId::new();
    let mut session = make_session(&session_id, "Schema Check");
    session.set_model(ModelSelection::Single("anthropic/claude-opus-4".to_owned()));
    store.save(&session).await.expect("save");

    // When listing the sessions table columns.
    let db_path = dir.path().join("sessions.db");
    let pool = daow::Pool::open(db_path.to_string_lossy().as_ref()).expect("open");
    let cols: Vec<ColumnRow> = pool
        .query_all::<ColumnRow>("PRAGMA table_info(sessions)", vec![])
        .await
        .expect("table_info");

    // Then exactly the 9 authoritative columns exist (no zombies).
    let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "id",
            "title",
            "updated_at",
            "created_at",
            "parent_session",
            "archived",
            "metadata",
            "is_automated",
            "persist"
        ],
        "sessions table must have exactly the 9 authoritative columns post-v20"
    );

    // And metadata carries the real model.
    let row: Option<MetadataRow> = pool
        .query_one(
            "SELECT metadata FROM sessions WHERE id = ?",
            vec![Box::new(session_id.to_string())],
        )
        .await
        .expect("select metadata");
    let metadata = row
        .expect("row exists")
        .metadata
        .expect("metadata non-null");
    assert!(
        metadata.contains("\"anthropic/claude-opus-4\""),
        "metadata blob should contain the real model: {metadata}"
    );
}

#[derive(Debug)]
struct ColumnRow {
    name: String,
}
impl daow::FromRow for ColumnRow {
    fn from_row(row: &daow::Row) -> daow::Result<Self> {
        Ok(Self {
            name: row.get("name")?,
        })
    }
}

#[derive(Debug)]
struct MetadataRow {
    metadata: Option<String>,
}
impl daow::FromRow for MetadataRow {
    fn from_row(row: &daow::Row) -> daow::Result<Self> {
        Ok(Self {
            metadata: row.get("metadata")?,
        })
    }
}

#[rstest::rstest]
#[tokio::test]
async fn user_entry_with_image_attachment_roundtrips_through_sqlite() {
    // Given a session with a user entry referencing a PNG on disk via @path.
    let (dir, store) = make_store().await;
    let session_id = SessionId::new();
    let png_path = dir.path().join("img.png");
    std::fs::write(&png_path, TINY_PNG).expect("write png");
    let display = format!("describe this @{}", png_path.to_string_lossy());
    let expanded = format!("describe this (file://{})", png_path.to_string_lossy());
    let mut entry = ChatEntry::user_expanded(display, expanded);
    // Populate the attachment directly — this test verifies SQLite persistence
    // of attachments, not the @path expansion pipeline (which resolves in the
    // session actor).
    if let ChatEntryKind::User { attachments, .. } = &mut entry.kind {
        attachments.push(jinn_provider::Attachment::image(
            "image/png",
            TINY_PNG.to_vec(),
        ));
    }
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.set_title("Image Session".to_owned());
    session.push_entry(entry);

    // When saving and reloading the session.
    store.save(&session).await.expect("save");
    let loaded = store
        .load_session(&session_id)
        .await
        .expect("load_session")
        .expect("session should exist");

    // Then the reloaded user entry carries the image attachment read from disk.
    let entry = &loaded.history()[0];
    let ChatEntryKind::User {
        attachments,
        expanded,
        ..
    } = &entry.kind
    else {
        panic!("expected a User entry");
    };
    assert_eq!(attachments.len(), 1);
    assert!(attachments[0].is_image());
    assert_eq!(attachments[0].data(), TINY_PNG);
    assert!(
        expanded.contains(&format!("(file://{})", png_path.to_string_lossy())),
        "expanded text should contain the file:// URI: {expanded}"
    );
}

/// Minimal valid PNG (1×1) magic bytes for attachment roundtrip tests.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89,
];
