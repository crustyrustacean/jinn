//! Tests for [`HistoryMutation`] application and queue operations.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ContextOverride, PinPosition};
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::history_mutation::HistoryMutation;

// --- find_entry_index_by_id ---

#[test]
fn find_entry_index_by_id_returns_index_for_existing_entry() {
    // Given a session with two entries.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    let second = ChatEntry::user("second");
    let second_id = second.id.clone();
    session.push_entry(second);

    // When resolving the second entry's ID.
    let idx = session.find_entry_index_by_id(&second_id);

    // Then it returns index 1.
    assert_eq!(idx, Some(1));
}

#[test]
fn find_entry_index_by_id_returns_none_for_nonexistent_id() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // When resolving a made-up ID.
    let fake_id = ChatEntryId::new();
    let idx = session.find_entry_index_by_id(&fake_id);

    // Then it returns None.
    assert!(idx.is_none());
}

// --- queue_mutations ---

#[test]
fn queue_mutations_appends_nonempty_batch() {
    // Given a session.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();

    let batch = vec![HistoryMutation::SetContextOverride {
        entry_id,
        value: ContextOverride::ForcedExclude,
    }];

    // When queuing a non-empty batch.
    session.queue_mutations(batch);

    // Then the queue has one batch.
    assert_eq!(session.drain_pending_mutations().len(), 1);
}

#[test]
fn queue_mutations_ignores_empty_batch() {
    // Given a session.
    let mut session = ChatSessionState::new();

    // When queuing an empty batch.
    session.queue_mutations(vec![]);

    // Then the queue is still empty.
    assert!(session.drain_pending_mutations().is_empty());
}

// --- drain_pending_mutations ---

#[test]
fn drain_pending_mutations_empties_the_queue() {
    // Given a session with one queued batch.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();

    session.queue_mutations(vec![HistoryMutation::SetContextOverride {
        entry_id,
        value: ContextOverride::ForcedExclude,
    }]);

    // When draining.
    let drained = session.drain_pending_mutations();

    // Then one batch is returned.
    assert_eq!(drained.len(), 1);

    // And the queue is now empty.
    assert!(session.drain_pending_mutations().is_empty());
}

// --- apply_mutations: SetContextOverride ---

#[test]
fn apply_mutations_sets_context_override_by_id() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();

    // When applying a SetContextOverride mutation.
    session.apply_mutations(vec![HistoryMutation::SetContextOverride {
        entry_id,
        value: ContextOverride::ForcedExclude,
    }]);

    // Then the entry's context override is set.
    assert_eq!(
        session.history()[0].context_override,
        ContextOverride::ForcedExclude
    );
}

#[test]
fn apply_mutations_skips_set_context_override_for_nonexistent_id() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // When applying a SetContextOverride mutation targeting a nonexistent ID.
    session.apply_mutations(vec![HistoryMutation::SetContextOverride {
        entry_id: ChatEntryId::new(),
        value: ContextOverride::ForcedExclude,
    }]);

    // Then the existing entry is unchanged.
    assert_eq!(
        session.history()[0].context_override,
        ContextOverride::Default
    );
}

// --- apply_mutations: InsertEntry ---

#[test]
fn apply_mutations_inserts_entry_after_specified_id() {
    // Given a session with two entries.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::user("third"));
    let first_id = session.history()[0].id.clone();

    let inserted = ChatEntry::system("second");
    let inserted_id = inserted.id.clone();

    // When inserting after the first entry.
    session.apply_mutations(vec![HistoryMutation::InsertEntry {
        after_entry_id: Some(first_id),
        entry: inserted,
    }]);

    // Then the new entry is at index 1.
    assert_eq!(session.history().len(), 3);
    assert_eq!(session.history()[1].id, inserted_id);
}

#[test]
fn apply_mutations_inserts_at_beginning_when_after_id_is_none() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("second"));
    let inserted = ChatEntry::system("first");
    let inserted_id = inserted.id.clone();

    // When inserting with after_entry_id = None.
    session.apply_mutations(vec![HistoryMutation::InsertEntry {
        after_entry_id: None,
        entry: inserted,
    }]);

    // Then the new entry is at index 0.
    assert_eq!(session.history().len(), 2);
    assert_eq!(session.history()[0].id, inserted_id);
}

#[test]
fn apply_mutations_skips_insert_for_nonexistent_after_id() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // When inserting after a nonexistent ID.
    session.apply_mutations(vec![HistoryMutation::InsertEntry {
        after_entry_id: Some(ChatEntryId::new()),
        entry: ChatEntry::system("ghost"),
    }]);

    // Then no entry was inserted.
    assert_eq!(session.history().len(), 1);
}

#[test]
fn insert_entry_shifts_streaming_entry_index() {
    // Given a session with streaming state pointing at index 1.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::user("second"));
    let first_id = session.history()[0].id.clone();
    session.begin_sending();
    session.begin_streaming();
    session.core.ephemeral.machine.set_streaming_entry_index(1);

    // When inserting before the streaming entry.
    session.apply_mutations(vec![HistoryMutation::InsertEntry {
        after_entry_id: Some(first_id),
        entry: ChatEntry::system("inserted"),
    }]);

    // Then streaming_entry_index is shifted to 2.
    assert_eq!(
        session.core.ephemeral.machine.streaming_entry_index(),
        Some(2)
    );
}

#[test]
fn insert_entry_shifts_streaming_thinking_entry_index() {
    // Given a session with thinking entry at index 1.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::user("second"));
    let first_id = session.history()[0].id.clone();
    session.begin_sending();
    session.begin_streaming();
    session
        .core
        .ephemeral
        .machine
        .set_streaming_thinking_entry_index(1);

    // When inserting before the thinking entry.
    session.apply_mutations(vec![HistoryMutation::InsertEntry {
        after_entry_id: Some(first_id),
        entry: ChatEntry::system("inserted"),
    }]);

    // Then streaming_thinking_entry_index is shifted to 2.
    assert_eq!(
        session
            .core
            .ephemeral
            .machine
            .streaming_thinking_entry_index(),
        Some(2)
    );
}

#[test]
fn insert_entry_shifts_streaming_tool_call_indices() {
    // Given a session with a tool call tracked at index 1.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::user("second"));
    let first_id = session.history()[0].id.clone();
    session.begin_sending();
    session.begin_streaming();
    session
        .core
        .ephemeral
        .machine
        .streaming_tool_call_indices_mut()
        .expect("streaming")
        .insert(0, 1);

    // When inserting before the tool call entry.
    session.apply_mutations(vec![HistoryMutation::InsertEntry {
        after_entry_id: Some(first_id),
        entry: ChatEntry::system("inserted"),
    }]);

    // Then the tool call index is shifted to 2.
    assert_eq!(
        session.core.ephemeral.machine.streaming_tool_call_indices()[&0],
        2
    );
}

#[test]
fn insert_entry_shifts_streaming_tool_result_indices() {
    // Given a session with a tool result tracked at index 1.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::user("second"));
    let first_id = session.history()[0].id.clone();
    session.begin_sending();
    session.begin_streaming();
    session
        .core
        .ephemeral
        .machine
        .streaming_tool_result_indices_mut()
        .expect("streaming")
        .insert("call_123".to_owned(), 1);

    // When inserting before the tool result entry.
    session.apply_mutations(vec![HistoryMutation::InsertEntry {
        after_entry_id: Some(first_id),
        entry: ChatEntry::system("inserted"),
    }]);

    // Then the tool result index is shifted to 2.
    assert_eq!(
        session
            .core
            .ephemeral
            .machine
            .streaming_tool_result_indices()["call_123"],
        2
    );
}

#[test]
fn multiple_insert_entries_in_one_batch_produce_correct_ordering() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("anchor"));
    let anchor_id = session.history()[0].id.clone();

    let first_insert = ChatEntry::system("A");
    let first_insert_id = first_insert.id.clone();
    let second_insert = ChatEntry::system("B");
    let second_insert_id = second_insert.id.clone();

    // When inserting two entries after the anchor in one batch.
    // First insert goes after anchor (at index 1).
    // Second insert also goes after anchor (at index 1), pushing the first insert to index 2.
    session.apply_mutations(vec![
        HistoryMutation::InsertEntry {
            after_entry_id: Some(anchor_id.clone()),
            entry: first_insert,
        },
        HistoryMutation::InsertEntry {
            after_entry_id: Some(anchor_id),
            entry: second_insert,
        },
    ]);

    // Then both entries are present: anchor, B, A (second insert pushed first forward).
    assert_eq!(session.history().len(), 3);
    assert_eq!(session.history()[1].id, second_insert_id);
    assert_eq!(session.history()[2].id, first_insert_id);
}

// --- apply_mutations: PinEntry ---

#[test]
fn apply_mutations_pins_entry_by_id() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();

    // When applying a PinEntry mutation.
    session.apply_mutations(vec![HistoryMutation::PinEntry {
        entry_id,
        position: PinPosition::Top,
    }]);

    // Then the entry is pinned.
    assert_eq!(session.history()[0].pin_position, Some(PinPosition::Top));
}

#[test]
fn apply_mutations_skips_pin_for_nonexistent_id() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // When pinning a nonexistent ID.
    session.apply_mutations(vec![HistoryMutation::PinEntry {
        entry_id: ChatEntryId::new(),
        position: PinPosition::Top,
    }]);

    // Then the existing entry is still unpinned.
    assert!(session.history()[0].pin_position.is_none());
}

// --- apply_mutations: UnpinEntry ---

#[test]
fn apply_mutations_unpins_entry_by_id() {
    // Given a session with a pinned entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello").with_pin(PinPosition::Top));
    let entry_id = session.history()[0].id.clone();

    // When applying an UnpinEntry mutation.
    session.apply_mutations(vec![HistoryMutation::UnpinEntry { entry_id }]);

    // Then the entry is unpinned.
    assert!(session.history()[0].pin_position.is_none());
}

// --- apply_mutations: edge cases ---

#[test]
fn apply_mutations_on_empty_history_is_noop() {
    // Given an empty session.
    let mut session = ChatSessionState::new();

    // When applying mutations targeting nonexistent entries.
    session.apply_mutations(vec![
        HistoryMutation::SetContextOverride {
            entry_id: ChatEntryId::new(),
            value: ContextOverride::ForcedExclude,
        },
        HistoryMutation::InsertEntry {
            after_entry_id: Some(ChatEntryId::new()),
            entry: ChatEntry::system("ghost"),
        },
        HistoryMutation::PinEntry {
            entry_id: ChatEntryId::new(),
            position: PinPosition::Top,
        },
        HistoryMutation::UnpinEntry {
            entry_id: ChatEntryId::new(),
        },
    ]);

    // Then nothing happened.
    assert!(session.history().is_empty());
}

// --- drain_and_apply_pending_mutations ---

#[test]
fn drain_and_apply_applies_all_batches_in_order() {
    // Given a session with two entries and two queued batches.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::user("second"));
    let first_id = session.history()[0].id.clone();
    let second_id = session.history()[1].id.clone();

    // Batch 1: exclude the first entry.
    session.queue_mutations(vec![HistoryMutation::SetContextOverride {
        entry_id: first_id,
        value: ContextOverride::ForcedExclude,
    }]);

    // Batch 2: exclude the second entry.
    session.queue_mutations(vec![HistoryMutation::SetContextOverride {
        entry_id: second_id,
        value: ContextOverride::ForcedExclude,
    }]);

    // When draining and applying.
    let count = session.drain_and_apply_pending_mutations();

    // Then both batches were applied.
    assert_eq!(count, 2);
    assert_eq!(
        session.history()[0].context_override,
        ContextOverride::ForcedExclude
    );
    assert_eq!(
        session.history()[1].context_override,
        ContextOverride::ForcedExclude
    );

    // And the queue is empty.
    assert!(session.drain_pending_mutations().is_empty());
}

#[test]
fn drain_and_apply_returns_zero_when_queue_empty() {
    // Given a session with no queued mutations.
    let mut session = ChatSessionState::new();

    // When draining and applying.
    let count = session.drain_and_apply_pending_mutations();

    // Then zero batches were applied.
    assert_eq!(count, 0);
}

// --- PinEntry with specific positions ---

#[test]
fn pin_entry_top_sets_position() {
    // Given a session with an entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();

    // When applying a PinEntry mutation with Top position.
    session.apply_mutations(vec![HistoryMutation::PinEntry {
        entry_id: entry_id.clone(),
        position: PinPosition::Top,
    }]);

    // Then the entry is pinned to Top.
    assert_eq!(session.history()[0].pin_position, Some(PinPosition::Top));
}

#[test]
fn pin_entry_bottom_sets_position() {
    // Given a session with an entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();

    // When applying a PinEntry mutation with Bottom position.
    session.apply_mutations(vec![HistoryMutation::PinEntry {
        entry_id: entry_id.clone(),
        position: PinPosition::Bottom,
    }]);

    // Then the entry is pinned to Bottom.
    assert_eq!(session.history()[0].pin_position, Some(PinPosition::Bottom));
}

#[test]
fn pin_entry_relative_sets_position() {
    // Given a session with an entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();

    // When applying a PinEntry mutation with Relative position.
    session.apply_mutations(vec![HistoryMutation::PinEntry {
        entry_id: entry_id.clone(),
        position: PinPosition::Relative,
    }]);

    // Then the entry is pinned to Relative.
    assert_eq!(
        session.history()[0].pin_position,
        Some(PinPosition::Relative)
    );
}

#[test]
fn pin_entry_can_change_position() {
    // Given a session with a Top-pinned entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();
    session.apply_mutations(vec![HistoryMutation::PinEntry {
        entry_id: entry_id.clone(),
        position: PinPosition::Top,
    }]);
    assert_eq!(session.history()[0].pin_position, Some(PinPosition::Top));

    // When changing to Bottom.
    session.apply_mutations(vec![HistoryMutation::PinEntry {
        entry_id: entry_id.clone(),
        position: PinPosition::Bottom,
    }]);

    // Then the position is updated.
    assert_eq!(session.history()[0].pin_position, Some(PinPosition::Bottom));
}

// --- ContextOverride variants ---

#[test]
fn set_context_override_forced_include() {
    // Given a session with an entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();

    // When applying SetContextOverride with ForcedInclude.
    session.apply_mutations(vec![HistoryMutation::SetContextOverride {
        entry_id: entry_id.clone(),
        value: ContextOverride::ForcedInclude,
    }]);

    // Then the entry has ForcedInclude.
    assert_eq!(
        session.history()[0].context_override,
        ContextOverride::ForcedInclude
    );
}

#[test]
fn set_context_override_default_resets_to_default() {
    // Given a session with a ForcedExclude entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();
    session.apply_mutations(vec![HistoryMutation::SetContextOverride {
        entry_id: entry_id.clone(),
        value: ContextOverride::ForcedExclude,
    }]);
    assert_eq!(
        session.history()[0].context_override,
        ContextOverride::ForcedExclude
    );

    // When resetting to Default.
    session.apply_mutations(vec![HistoryMutation::SetContextOverride {
        entry_id: entry_id.clone(),
        value: ContextOverride::Default,
    }]);

    // Then the entry is back to Default.
    assert_eq!(
        session.history()[0].context_override,
        ContextOverride::Default
    );
}

#[test]
fn unpin_entry_removes_pin_from_pinned_entry() {
    // Given a session with a pinned entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();
    session.apply_mutations(vec![HistoryMutation::PinEntry {
        entry_id: entry_id.clone(),
        position: PinPosition::Top,
    }]);
    assert!(session.history()[0].pin_position.is_some());

    // When applying UnpinEntry.
    session.apply_mutations(vec![HistoryMutation::UnpinEntry {
        entry_id: entry_id.clone(),
    }]);

    // Then the pin is removed.
    assert!(session.history()[0].pin_position.is_none());
}

#[test]
fn unpin_entry_on_unpinned_entry_is_noop() {
    // Given a session with an unpinned entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let entry_id = session.history()[0].id.clone();
    assert!(session.history()[0].pin_position.is_none());

    // When applying UnpinEntry on an already-unpinned entry.
    session.apply_mutations(vec![HistoryMutation::UnpinEntry {
        entry_id: entry_id.clone(),
    }]);

    // Then no panic, still None.
    assert!(session.history()[0].pin_position.is_none());
}
