use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::{ChatEntry, ChatEntryId, ChatEntryKind, PinPosition, PromptStrategyId};
use std::path::PathBuf;

use super::*;

#[rstest::rstest]
fn push_entry_adds_to_history() {
    // Given a new ChatSessionState.
    let mut session = ChatSessionState::new();

    // When pushing a user entry.
    let index = session.push_entry(ChatEntry::user("hello"));

    // Then the index is 0 and history has one entry.
    assert_eq!(index, 0);
    assert_eq!(session.history().len(), 1);
}

#[rstest::rstest]
fn first_stream_token_creates_assistant_entry() {
    // Given a session with one entry, streaming started.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    session.begin_streaming();

    // When appending the first token.
    session.append_stream_token("Hello");

    // Then the assistant entry is created.
    assert_eq!(session.history().len(), 2);
    assert!(matches!(
        session.history()[1].kind,
        ChatEntryKind::Assistant(ref text) if text == "Hello"
    ));
}

#[rstest::rstest]
fn begin_streaming_sets_is_streaming() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // When beginning streaming.
    session.begin_streaming();

    // Then is_streaming is true.
    assert!(session.is_streaming());
}

#[rstest::rstest]
fn append_stream_token_appends_to_assistant_entry() {
    // Given a session that is streaming.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When appending a token.
    session.append_stream_token("Hello");
    session.append_stream_token(" world");

    // Then the assistant entry text is "Hello world".
    assert_eq!(
        session.history()[0].kind,
        ChatEntryKind::Assistant("Hello world".to_owned())
    );
}

#[rstest::rstest]
fn finish_streaming_clears_streaming_state() {
    // Given a session that is streaming with some tokens.
    let mut session = ChatSessionState::new();
    session.begin_streaming();
    session.append_stream_token("Hi");

    // When finishing streaming.
    session.finish_streaming();

    // Then is_streaming is false and text is preserved.
    assert!(!session.is_streaming());
    assert_eq!(
        session.history()[0].kind,
        ChatEntryKind::Assistant("Hi".to_owned())
    );
}

#[rstest::rstest]
fn cancel_streaming_keeps_partial_text() {
    // Given a session that is streaming with partial tokens.
    let mut session = ChatSessionState::new();
    session.begin_streaming();
    session.append_stream_token("Partial");

    // When cancelling streaming.
    session.cancel_streaming();

    // Then is_streaming is false but partial text is kept.
    assert!(!session.is_streaming());
    assert_eq!(
        session.history()[0].kind,
        ChatEntryKind::Assistant("Partial".to_owned())
    );
}

#[rstest::rstest]
#[should_panic(expected = "begin_streaming called while already streaming")]
fn begin_streaming_twice_panics() {
    // Given a session that is already streaming.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When calling begin_streaming again.
    // Then it panics.
    session.begin_streaming();
}

#[rstest::rstest]
#[should_panic(expected = "append_stream_token called while not streaming")]
fn append_stream_token_when_not_streaming_panics() {
    // Given a session that is not streaming.
    let mut session = ChatSessionState::new();

    // When calling append_stream_token.
    // Then it panics.
    session.append_stream_token("oops");
}

#[rstest::rstest]
fn scroll_up_from_bottom_decrements_offset() {
    // Given a session at the bottom with last_max_offset = 100.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.reset_scroll();
    assert!(session.scroll_offset().is_none());

    // When scrolling up by 10.
    session.scroll_up(10);

    // Then the offset is 90 (100 − 10).
    assert_eq!(session.scroll_offset(), Some(90));
}

#[rstest::rstest]
fn scroll_up_from_known_offset_decrements() {
    // Given a session with scroll_offset = 50 and last_max_offset = 100.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.ui.scroll_offset = Some(50);

    // When scrolling up by 10.
    session.scroll_up(10);

    // Then the offset is 40.
    assert_eq!(session.scroll_offset(), Some(40));
}

#[rstest::rstest]
fn scroll_up_saturates_at_zero() {
    // Given a session with scroll_offset = 5 and last_max_offset = 100.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.ui.scroll_offset = Some(5);

    // When scrolling up by 20.
    session.scroll_up(20);

    // Then the offset saturates at 0.
    assert_eq!(session.scroll_offset(), Some(0));
}

#[rstest::rstest]
fn scroll_down_increments_offset() {
    // Given a session with scroll_offset = 0 and last_max_offset = 100.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.ui.scroll_offset = Some(0);

    // When scrolling down by 10.
    session.scroll_down(10);

    // Then the offset increased by 10.
    assert_eq!(session.scroll_offset(), Some(10));
}

#[rstest::rstest]
fn scroll_down_past_bottom_resets_to_auto() {
    // Given a session with scroll_offset = 95 and last_max_offset = 100.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.ui.scroll_offset = Some(95);

    // When scrolling down by 10.
    session.scroll_down(10);

    // Then the offset resets to None (auto-scroll to bottom).
    assert!(session.scroll_offset().is_none());
}

#[rstest::rstest]
fn scroll_to_top_sets_offset_to_zero() {
    // Given a session scrolled to the middle.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.ui.scroll_offset = Some(50);

    // When scrolling to top.
    session.scroll_to_top();

    // Then the offset is 0.
    assert_eq!(session.scroll_offset(), Some(0));
}

#[rstest::rstest]
fn scroll_to_bottom_resets_to_auto_scroll() {
    // Given a session scrolled to the top.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.ui.scroll_offset = Some(0);

    // When scrolling to bottom.
    session.scroll_to_bottom();

    // Then the offset is None (auto-scroll).
    assert!(session.scroll_offset().is_none());
}

#[rstest::rstest]
fn reset_scroll_clears_offset() {
    // Given a session with scroll_offset = 50.
    let mut session = ChatSessionState::new();
    session.ui.scroll_offset = Some(50);

    // When resetting scroll.
    session.reset_scroll();

    // Then the offset is None (at bottom).
    assert!(session.scroll_offset().is_none());
}

#[rstest::rstest]
fn push_entry_resets_scroll() {
    // Given a session with scroll_offset = 50.
    let mut session = ChatSessionState::new();
    session.ui.scroll_offset = Some(50);

    // When pushing an entry.
    session.push_entry(ChatEntry::user("hello"));

    // Then scroll_offset is None (reset by push_entry).
    assert!(session.scroll_offset().is_none());
}

#[rstest::rstest]
fn is_at_bottom_true_when_auto_scroll() {
    // Given a new session (auto-scroll to bottom).
    let session = ChatSessionState::new();

    // Then is_at_bottom is true.
    assert!(session.is_at_bottom());
}

#[rstest::rstest]
fn is_at_bottom_false_when_scrolled_up() {
    // Given a session scrolled to offset 50.
    let mut session = ChatSessionState::new();
    session.ui.scroll_offset = Some(50);

    // Then is_at_bottom is false.
    assert!(!session.is_at_bottom());
}

// --- Queue tests ---

#[rstest::rstest]
fn enqueue_message_adds_to_queue() {
    // Given a new session with an empty queue.
    let mut session = ChatSessionState::new();
    assert_eq!(session.queue_len(), 0);

    // When enqueuing a message.
    session.enqueue_message(ChatEntry::user("hello"));

    // Then the queue has one message.
    assert_eq!(session.queue_len(), 1);
    assert_eq!(
        session.queue()[0].kind,
        ChatEntryKind::User {
            display: "hello".to_owned(),
            expanded: "hello".to_owned()
        }
    );
}

#[rstest::rstest]
fn dequeue_message_returns_first_in_order() {
    // Given a session with two queued messages.
    let mut session = ChatSessionState::new();
    session.enqueue_message(ChatEntry::user("first"));
    session.enqueue_message(ChatEntry::user("second"));

    // When dequeuing a message.
    let msg = session.dequeue_message();

    // Then it returns the first message and the queue has one left.
    assert!(msg.is_some());
    let entry = msg.unwrap();
    assert_eq!(
        entry.kind,
        ChatEntryKind::User {
            display: "first".to_owned(),
            expanded: "first".to_owned()
        }
    );
    assert_eq!(session.queue_len(), 1);
}

#[rstest::rstest]
fn dequeue_message_returns_none_when_empty() {
    // Given a session with an empty queue.
    let mut session = ChatSessionState::new();

    // When dequeuing a message.
    let msg = session.dequeue_message();

    // Then it returns None.
    assert!(msg.is_none());
}

#[rstest::rstest]
fn drain_returns_all_in_order() {
    // Given a session with three queued messages.
    let mut session = ChatSessionState::new();
    session.enqueue_message(ChatEntry::user("a"));
    session.enqueue_message(ChatEntry::user("b"));
    session.enqueue_message(ChatEntry::user("c"));

    // When draining the queue.
    let drained = session.drain_queue();

    // Then all messages are returned in order.
    assert_eq!(drained.len(), 3);
    assert_eq!(
        drained[0].kind,
        ChatEntryKind::User {
            display: "a".to_owned(),
            expanded: "a".to_owned()
        }
    );
    assert_eq!(
        drained[1].kind,
        ChatEntryKind::User {
            display: "b".to_owned(),
            expanded: "b".to_owned()
        }
    );
    assert_eq!(
        drained[2].kind,
        ChatEntryKind::User {
            display: "c".to_owned(),
            expanded: "c".to_owned()
        }
    );
}

#[rstest::rstest]
fn drain_empties_queue() {
    // Given a session with three queued messages.
    let mut session = ChatSessionState::new();
    session.enqueue_message(ChatEntry::user("a"));
    session.enqueue_message(ChatEntry::user("b"));
    session.enqueue_message(ChatEntry::user("c"));

    // When draining the queue.
    let _ = session.drain_queue();

    // Then the queue is empty.
    assert_eq!(session.queue_len(), 0);
}

// --- Sending tests ---

#[rstest::rstest]
fn begin_sending_sets_is_sending() {
    // Given a new session (idle).
    let mut session = ChatSessionState::new();
    assert!(!session.is_sending());

    // When beginning sending.
    session.begin_sending();

    // Then is_sending is true.
    assert!(session.is_sending());
}

#[rstest::rstest]
#[should_panic(expected = "begin_sending called while already sending or streaming")]
fn begin_sending_panics_when_already_sending() {
    // Given a session that is already sending.
    let mut session = ChatSessionState::new();
    session.begin_sending();

    // When calling begin_sending again.
    // Then it panics.
    session.begin_sending();
}

#[rstest::rstest]
#[should_panic(expected = "begin_sending called while already sending or streaming")]
fn begin_sending_panics_when_streaming() {
    // Given a session that is streaming.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When calling begin_sending.
    // Then it panics.
    session.begin_sending();
}

#[rstest::rstest]
fn finish_sending_clears_flag() {
    // Given a session that is sending.
    let mut session = ChatSessionState::new();
    session.begin_sending();

    // When finishing sending.
    session.finish_sending();

    // Then is_sending is false.
    assert!(!session.is_sending());
}

#[rstest::rstest]
#[should_panic(expected = "finish_sending called while not sending")]
fn finish_sending_panics_when_not_sending() {
    // Given a session that is not sending.
    let mut session = ChatSessionState::new();

    // When calling finish_sending.
    // Then it panics.
    session.finish_sending();
}

// --- Combined status tests ---

#[rstest::rstest]
fn is_idle_true_when_not_sending_or_streaming() {
    // Given a fresh session.
    let session = ChatSessionState::new();

    // Then it is idle.
    assert!(session.is_idle());
}

#[rstest::rstest]
fn is_idle_false_when_sending() {
    // Given a session that is sending.
    let mut session = ChatSessionState::new();
    session.begin_sending();

    // Then it is not idle.
    assert!(!session.is_idle());
}

#[rstest::rstest]
fn is_idle_false_when_streaming() {
    // Given a session that is streaming.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // Then it is not idle.
    assert!(!session.is_idle());
}

#[rstest::rstest]
fn cancel_streaming_clears_is_streaming() {
    // Given a session that was sending before streaming started.
    let mut session = ChatSessionState::new();
    session.begin_sending();
    session.core.ephemeral.is_streaming = true;
    assert!(session.is_streaming());

    // When cancelling streaming.
    session.cancel_streaming();

    // Then is_streaming is cleared.
    assert!(!session.is_streaming());
}

#[rstest::rstest]
fn cancel_streaming_clears_is_sending() {
    // Given a session that was sending before streaming started.
    let mut session = ChatSessionState::new();
    session.begin_sending();
    session.core.ephemeral.is_streaming = true;
    assert!(session.is_sending());

    // When cancelling streaming.
    session.cancel_streaming();

    // Then is_sending is cleared.
    assert!(!session.is_sending());
}

#[rstest::rstest]
fn finish_streaming_clears_is_sending() {
    // Given a session that was sending before streaming started.
    let mut session = ChatSessionState::new();
    session.begin_sending();
    // Manually set is_streaming to simulate the transition.
    session.core.ephemeral.is_streaming = true;
    session.core.ephemeral.streaming_entry_index =
        Some(session.push_entry(ChatEntry::assistant("")));

    // When finishing streaming.
    session.finish_streaming();

    // Then is_sending is cleared.
    assert!(!session.is_sending());
}

#[rstest::rstest]
fn finish_streaming_clears_is_streaming() {
    // Given a session that was sending before streaming started.
    let mut session = ChatSessionState::new();
    session.begin_sending();
    // Manually set is_streaming to simulate the transition.
    session.core.ephemeral.is_streaming = true;
    session.core.ephemeral.streaming_entry_index =
        Some(session.push_entry(ChatEntry::assistant("")));

    // When finishing streaming.
    session.finish_streaming();

    // Then is_streaming is cleared.
    assert!(!session.is_streaming());
}

// --- Tool call streaming tests ---

#[rstest::rstest]
fn begin_tool_call_creates_entry_with_empty_arguments() {
    // Given a streaming session.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When beginning a tool call.
    session.begin_tool_call(0, "call_1", "echo");

    // Then history has an assistant entry and a tool call entry with empty arguments.
    assert_eq!(session.history().len(), 2);
    assert!(matches!(
        session.history()[0].kind,
        ChatEntryKind::Assistant(_)
    ));
    assert_eq!(
        session.history()[1].kind,
        ChatEntryKind::ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: String::new(),
        }
    );
}

#[rstest::rstest]
fn append_tool_call_delta_accumulates_arguments() {
    // Given a streaming session with a tool call entry.
    let mut session = ChatSessionState::new();
    session.begin_streaming();
    session.begin_tool_call(0, "call_1", "echo");

    // When appending tool call deltas.
    session.append_tool_call_delta(0, r#"{"input":"#);
    session.append_tool_call_delta(0, r#""hello"}"#);

    // Then the tool call entry has the accumulated arguments.
    assert_eq!(
        session.history()[1].kind,
        ChatEntryKind::ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hello"}"#.to_owned(),
        }
    );
}

#[rstest::rstest]
fn finalize_tool_call_overwrites_arguments() {
    // Given a streaming session with a tool call that has partial arguments.
    let mut session = ChatSessionState::new();
    session.begin_streaming();
    session.begin_tool_call(0, "call_1", "echo");
    session.append_tool_call_delta(0, r#"{"input":"#);

    // When finalizing the tool call with the complete arguments.
    session.finalize_tool_call("call_1", "echo", r#"{"input":"world"}"#);

    // Then the arguments are overwritten with the final value.
    assert_eq!(
        session.history()[1].kind,
        ChatEntryKind::ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"world"}"#.to_owned(),
        }
    );
}

#[rstest::rstest]
fn finalize_tool_call_pushes_new_entry_when_not_found() {
    // Given a streaming session with no tool call entry for the given ID.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When finalizing a tool call that was never started (shouldn't happen normally).
    session.finalize_tool_call("call_99", "echo", r#"{"input":"hi"}"#);

    // Then a new entry is pushed (no assistant entry yet \xe2\x80\x94 lazy creation).
    assert_eq!(session.history().len(), 1); // tool call only
    assert_eq!(
        session.history()[0].kind,
        ChatEntryKind::ToolCall {
            id: "call_99".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        }
    );
}

#[rstest::rstest]
fn first_tool_call_tracks_arguments() {
    // Given a streaming session.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When beginning a tool call and appending a delta.
    session.begin_tool_call(0, "call_1", "echo");
    session.append_tool_call_delta(0, r#"{"a":1}"#);

    // Then the tool call entry tracks its own arguments.
    assert_eq!(
        session.history()[1].kind,
        ChatEntryKind::ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"a":1}"#.to_owned(),
        }
    );
}

#[rstest::rstest]
fn second_tool_call_tracks_independent_arguments() {
    // Given a streaming session with one tool call already started.
    let mut session = ChatSessionState::new();
    session.begin_streaming();
    session.begin_tool_call(0, "call_1", "echo");
    session.append_tool_call_delta(0, r#"{"a":1}"#);

    // When beginning a second tool call with a different index.
    session.begin_tool_call(1, "call_2", "get_time");
    session.append_tool_call_delta(1, "{}");

    // Then the second tool call entry tracks its own arguments independently.
    assert_eq!(
        session.history()[2].kind,
        ChatEntryKind::ToolCall {
            id: "call_2".to_owned(),
            name: "get_time".to_owned(),
            arguments: "{}".to_owned(),
        }
    );
}

#[rstest::rstest]
fn finish_streaming_clears_tool_call_indices() {
    // Given a streaming session with a tool call entry.
    let mut session = ChatSessionState::new();
    session.begin_streaming();
    session.begin_tool_call(0, "call_1", "echo");

    // When finishing streaming.
    session.finish_streaming();

    // Then the tool call indices are cleared (entries remain in history).
    assert!(!session.is_streaming());
    assert_eq!(session.history().len(), 2); // assistant + tool call still there
}

#[rstest::rstest]
fn cancel_streaming_clears_tool_call_indices() {
    // Given a streaming session with a tool call entry.
    let mut session = ChatSessionState::new();
    session.begin_streaming();
    session.begin_tool_call(0, "call_1", "echo");

    // When cancelling streaming.
    session.cancel_streaming();

    // Then the tool call indices are cleared (entries remain in history).
    assert!(!session.is_streaming());
    assert_eq!(session.history().len(), 2); // assistant + tool call still there
}

// --- Strategy switching tests ---

#[rstest::rstest]
fn default_strategy_is_passthrough() {
    // Given a new session.
    let session = ChatSessionState::new();

    // Then the default strategy is passthrough.
    assert_eq!(session.active_strategy(), &PromptStrategyId::passthrough());
}

#[rstest::rstest]
fn switch_strategy_updates_active_strategy() {
    // Given a new session.
    let mut session = ChatSessionState::new();

    // When switching to sliding_window.
    session.switch_strategy(PromptStrategyId::sliding_window());

    // Then the active strategy is updated.
    assert_eq!(
        session.active_strategy(),
        &PromptStrategyId::sliding_window()
    );
}

#[rstest::rstest]
fn new_with_strategy_sets_active_strategy() {
    // Given a strategy ID.
    let strategy = PromptStrategyId::sliding_window();

    // When creating a session with that strategy.
    let session = ChatSessionState::new_with_strategy(strategy.clone());

    // Then the active strategy is set to the given strategy.
    assert_eq!(session.active_strategy(), &strategy);
}

#[rstest::rstest]
fn new_with_strategy_creates_empty_history() {
    // Given any strategy.
    let strategy = PromptStrategyId::compaction();

    // When creating a session with that strategy.
    let session = ChatSessionState::new_with_strategy(strategy);

    // Then the history is empty.
    assert!(session.history().is_empty());
}

// --- Pinning tests ---

#[rstest::rstest]
fn pin_state_sets_position() {
    // Given a session with two entries.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    let id0 = session.history()[0].id.clone();
    session.push_entry(ChatEntry::user("second"));

    // When pinning the first entry as Top.
    session.pin_entry(&id0, PinPosition::Top);

    // Then the first entry has pin_position set to Top.
    assert_eq!(session.history()[0].pin_position, Some(PinPosition::Top));
}

#[rstest::rstest]
fn pin_state_does_not_affect_other_entries() {
    // Given a session with two entries.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    let id0 = session.history()[0].id.clone();
    session.push_entry(ChatEntry::user("second"));

    // When pinning the first entry as Top.
    session.pin_entry(&id0, PinPosition::Top);

    // Then the second entry is still unpinned.
    assert_eq!(session.history()[1].pin_position, None);
}

#[rstest::rstest]
fn pin_entry_is_noop_for_nonexistent_id() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // When pinning a random ID.
    let fake_id = ChatEntryId::new();
    session.pin_entry(&fake_id, PinPosition::Top);

    // Then no entries changed.
    assert_eq!(session.history()[0].pin_position, None);
}

#[rstest::rstest]
fn unpin_entry_clears_position() {
    // Given a session with a pinned entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("test"));
    let id = session.history()[0].id.clone();
    session.pin_entry(&id, PinPosition::Top);
    assert_eq!(session.history()[0].pin_position, Some(PinPosition::Top));

    // When unpinning.
    session.unpin_entry(&id);

    // Then the pin position is cleared.
    assert_eq!(session.history()[0].pin_position, None);
}

#[rstest::rstest]
fn unpin_entry_is_noop_for_nonexistent_id() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // When unpinning a random ID.
    let fake_id = ChatEntryId::new();
    session.unpin_entry(&fake_id);

    // Then no panic and no entries changed.
    assert_eq!(session.history()[0].pin_position, None);
}

#[rstest::rstest]
fn pinned_entries_returns_only_pinned() {
    // Given a session with three entries, two pinned.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::user("second"));
    session.push_entry(ChatEntry::user("third"));
    let id0 = session.history()[0].id.clone();
    let id2 = session.history()[2].id.clone();
    session.pin_entry(&id0, PinPosition::Top);
    session.pin_entry(&id2, PinPosition::Bottom);

    // When getting pinned entries.
    let pinned = session.pinned_entries();

    // Then only the pinned entries are returned.
    assert_eq!(pinned.len(), 2);
    assert_eq!(pinned[0].id, id0);
    assert_eq!(pinned[1].id, id2);
}

#[rstest::rstest]
fn pinned_entries_returns_correct_count() {
    // Given a session with five entries, three pinned at indices 0, 2, 4.
    let mut session = ChatSessionState::new();
    for i in 0..5 {
        session.push_entry(ChatEntry::user(format!("msg {i}")));
    }
    let id0 = session.history()[0].id.clone();
    let id2 = session.history()[2].id.clone();
    let id4 = session.history()[4].id.clone();
    session.pin_entry(&id4, PinPosition::Relative);
    session.pin_entry(&id0, PinPosition::Top);
    session.pin_entry(&id2, PinPosition::Bottom);

    // When getting pinned entries.
    let pinned = session.pinned_entries();

    // Then three entries are returned.
    assert_eq!(pinned.len(), 3);
}

#[rstest::rstest]
fn pinned_entries_returns_in_order() {
    // Given a session with five entries, three pinned at indices 0, 2, 4.
    let mut session = ChatSessionState::new();
    for i in 0..5 {
        session.push_entry(ChatEntry::user(format!("msg {i}")));
    }
    let id0 = session.history()[0].id.clone();
    let id2 = session.history()[2].id.clone();
    let id4 = session.history()[4].id.clone();
    // Pin in reverse order to verify ordering is by history, not pin order.
    session.pin_entry(&id4, PinPosition::Relative);
    session.pin_entry(&id0, PinPosition::Top);
    session.pin_entry(&id2, PinPosition::Bottom);

    // When getting pinned entries.
    let pinned = session.pinned_entries();

    // Then they are in history order (0, 2, 4).
    assert_eq!(pinned[0].id, id0);
    assert_eq!(pinned[1].id, id2);
    assert_eq!(pinned[2].id, id4);
}

#[rstest::rstest]
fn pinned_entries_returns_empty_when_none_pinned() {
    // Given a session with entries, none pinned.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));

    // When getting pinned entries.
    let pinned = session.pinned_entries();

    // Then the result is empty.
    assert!(pinned.is_empty());
}

#[rstest::rstest]
fn pin_entry_can_change_position() {
    // Given a session with a pinned entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("test"));
    let id = session.history()[0].id.clone();
    session.pin_entry(&id, PinPosition::Top);
    assert_eq!(session.history()[0].pin_position, Some(PinPosition::Top));

    // When re-pinning with a different position.
    session.pin_entry(&id, PinPosition::Bottom);

    // Then the position is updated.
    assert_eq!(session.history()[0].pin_position, Some(PinPosition::Bottom));
}

#[rstest::rstest]
fn pin_position_survives_restore_history() {
    // Given a history with pinned entries.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a").with_pin(PinPosition::Top));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c").with_pin(PinPosition::Bottom));

    let original_history = session.history().to_vec();

    // When restoring history from a snapshot.
    let mut new_session = ChatSessionState::new();
    new_session.restore_history(original_history);

    // Then pinned entries survive.
    let pinned = new_session.pinned_entries();
    assert_eq!(pinned.len(), 2);
    assert_eq!(pinned[0].pin_position, Some(PinPosition::Top));
    assert_eq!(pinned[1].pin_position, Some(PinPosition::Bottom));
}

// --- Selection tests ---

#[rstest::rstest]
fn select_next_entry_starts_at_first_when_no_selection() {
    // Given a session with 3 entries and no selection.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    // push_entry auto-selects, so clear to test the "no selection" case.
    session.clear_selection();
    assert_eq!(session.selected_entry_index(), None);

    // When selecting next.
    session.select_next_entry();

    // Then the first entry (index 0) is selected.
    assert_eq!(session.selected_entry_index(), Some(0));
}

#[rstest::rstest]
fn select_next_entry_increments_from_current() {
    // Given a session with 3 entries and selection at index 1.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    session.select_next_entry(); // 0
    session.select_next_entry(); // 1

    // When selecting next again.
    session.select_next_entry();

    // Then the index is 2.
    assert_eq!(session.selected_entry_index(), Some(2));
}

#[rstest::rstest]
fn select_next_entry_clamps_at_last_index() {
    // Given a session with 3 entries and selection at last index.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    session.select_next_entry(); // 0
    session.select_next_entry(); // 1
    session.select_next_entry(); // 2

    // When selecting next again.
    session.select_next_entry();

    // Then the index stays at 2.
    assert_eq!(session.selected_entry_index(), Some(2));
}

#[rstest::rstest]
fn select_prev_entry_starts_at_last_when_no_selection() {
    // Given a session with 3 entries and no selection.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    // push_entry auto-selects last, so clear to test the "no selection" case.
    session.clear_selection();

    // When selecting prev.
    session.select_prev_entry();

    // Then the last entry (index 2) is selected.
    assert_eq!(session.selected_entry_index(), Some(2));
}

#[rstest::rstest]
fn select_prev_entry_decrements_from_current() {
    // Given a session with 3 entries and selection at index 2.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    // push_entry auto-selects last (index 2).
    assert_eq!(session.selected_entry_index(), Some(2));

    // When selecting prev again.
    session.select_prev_entry();

    // Then the index is 1.
    assert_eq!(session.selected_entry_index(), Some(1));
}

#[rstest::rstest]
fn select_prev_entry_clamps_at_zero() {
    // Given a session with 3 entries and selection at index 0.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    // push_entry auto-selects last (2). Move to index 0.
    session.ui.selected_entry_index = Some(0);

    // When selecting prev.
    session.select_prev_entry();

    // Then the index stays at 0.
    assert_eq!(session.selected_entry_index(), Some(0));
}

#[rstest::rstest]
fn select_next_is_noop_on_empty_history() {
    // Given an empty session.
    let mut session = ChatSessionState::new();

    // When selecting next.
    session.select_next_entry();

    // Then no selection is set.
    assert_eq!(session.selected_entry_index(), None);
}

#[rstest::rstest]
fn select_prev_is_noop_on_empty_history() {
    // Given an empty session.
    let mut session = ChatSessionState::new();

    // When selecting prev.
    session.select_prev_entry();

    // Then no selection is set.
    assert_eq!(session.selected_entry_index(), None);
}

#[rstest::rstest]
fn clear_selection_resets_to_none() {
    // Given a session with a selection.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    // push_entry auto-selects index 0.
    assert_eq!(session.selected_entry_index(), Some(0));

    // When clearing selection.
    session.clear_selection();

    // Then selection is None.
    assert_eq!(session.selected_entry_index(), None);
}

#[rstest::rstest]
fn selected_entry_returns_entry_at_index() {
    // Given a session with entries, second selected.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    // push_entry auto-selects last (2). Move to index 1.
    session.ui.selected_entry_index = Some(1);

    // When getting the selected entry.
    let entry = session.selected_entry();

    // Then it returns the entry at index 1.
    assert!(entry.is_some());
    assert_eq!(
        entry.unwrap().kind,
        ChatEntryKind::User {
            display: "b".to_owned(),
            expanded: "b".to_owned()
        }
    );
}

#[rstest::rstest]
fn selected_entry_id_returns_id_at_index() {
    // Given a session with a selected entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));
    let expected_id = session.history()[0].id.clone();
    // push_entry auto-selects index 0.

    // When getting the selected entry ID.
    let id = session.selected_entry_id();

    // Then it matches the first entry's ID.
    assert_eq!(id, Some(&expected_id));
}

#[rstest::rstest]
fn push_entry_auto_selects_new_entry_when_at_last() {
    // Given a session with a selected last entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    assert_eq!(session.selected_entry_index(), Some(0));

    // When pushing a new entry.
    session.push_entry(ChatEntry::user("b"));

    // Then the cursor advances to the new entry.
    assert_eq!(session.selected_entry_index(), Some(1));
}

#[rstest::rstest]
fn restore_history_auto_selects_last_entry() {
    // Given a session with a selected entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    assert_eq!(session.selected_entry_index(), Some(0));

    // When restoring history.
    session.restore_history(vec![ChatEntry::user("new")]);

    // Then the last entry is auto-selected.
    assert_eq!(session.selected_entry_index(), Some(0));
}

// --- Thinking streaming tests ---

#[rstest::rstest]
fn begin_thinking_appends_before_assistant_is_created() {
    // Given a streaming session (no assistant entry yet — lazy creation).
    let mut session = ChatSessionState::builder()
        .with_user_entry("hello")
        .begin_streaming()
        .build();
    // begin_streaming no longer creates an entry.
    assert_eq!(session.history().len(), 1);

    // When beginning thinking.
    session.begin_thinking();

    // Then the Thinking entry is appended (index 1).
    // No Assistant entry yet — it will be created on first token.
    assert_eq!(session.history().len(), 2);
    assert!(matches!(
        session.history()[1].kind,
        ChatEntryKind::Thinking(_)
    ));
    assert_eq!(session.streaming_thinking_entry_index(), Some(1));
}

#[rstest::rstest]
fn append_thinking_token_appends_to_thinking_entry() {
    // Given a session with a streaming Assistant entry that has begun thinking.
    let mut session = ChatSessionState::builder()
        .with_user_entry("hello")
        .begin_streaming()
        .build();
    session.begin_thinking();

    // When appending thinking tokens.
    session.append_thinking_token("reasoning");
    session.append_thinking_token(" more");

    // Then the Thinking entry has the accumulated text.
    match &session.history()[1].kind {
        ChatEntryKind::Thinking(text) => assert_eq!(text, "reasoning more"),
        other => panic!("expected Thinking, got {other:?}"),
    }
}

#[rstest::rstest]
fn finish_streaming_clears_thinking_entry_index() {
    // Given a session with a thinking entry and assistant entry.
    let mut session = ChatSessionState::builder()
        .with_user_entry("hello")
        .begin_streaming()
        .build();
    session.begin_thinking();
    session.append_thinking_token("reasoning");
    session.append_stream_token("response");

    // When finishing streaming.
    session.finish_streaming();

    // Then the thinking entry index is cleared.
    assert_eq!(session.streaming_thinking_entry_index(), None);
    // And the thinking text is preserved in history.
    assert!(
        matches!(session.history()[1].kind, ChatEntryKind::Thinking(ref t) if t == "reasoning")
    );
    assert!(
        matches!(session.history()[2].kind, ChatEntryKind::Assistant(ref t) if t == "response")
    );
}

#[rstest::rstest]
fn cancel_streaming_preserves_partial_thinking() {
    // Given a session with partial thinking text.
    let mut session = ChatSessionState::builder()
        .with_user_entry("hello")
        .begin_streaming()
        .build();
    session.begin_thinking();
    session.append_thinking_token("partial reasoning");

    // When cancelling streaming.
    session.cancel_streaming();

    // Then the partial thinking text is preserved.
    assert_eq!(session.streaming_thinking_entry_index(), None);
    assert!(
        matches!(session.history()[1].kind, ChatEntryKind::Thinking(ref t) if t == "partial reasoning")
    );
}

// --- Smart auto-scroll (was_at_last) tests ---

#[rstest::rstest]
fn push_entry_auto_selects_first_entry() {
    // Given an empty session.
    let mut session = ChatSessionState::new();

    // When pushing the first entry.
    session.push_entry(ChatEntry::user("hello"));

    // Then the new entry is auto-selected.
    assert_eq!(session.selected_entry_index(), Some(0));
    // And scroll is reset to bottom.
    assert!(session.is_at_bottom());
}

#[rstest::rstest]
fn push_entry_preserves_selection_when_not_at_last() {
    // Given a session with 3 entries, cursor on first.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    // Move cursor to first entry (not last).
    session.ui.selected_entry_index = Some(0);

    // When pushing a new entry.
    session.push_entry(ChatEntry::user("d"));

    // Then the cursor stays on index 0.
    assert_eq!(session.selected_entry_index(), Some(0));
}

#[rstest::rstest]
fn push_entry_resets_scroll_only_when_at_last() {
    // Given a session with entries, scrolled up.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    // Scroll up and move cursor away from last.
    session.ui.scroll_offset = Some(0);
    session.ui.selected_entry_index = Some(0);

    // When pushing a new entry.
    session.push_entry(ChatEntry::user("d"));

    // Then the scroll is NOT reset (cursor was not at last).
    assert_eq!(session.scroll_offset(), Some(0));
}

#[rstest::rstest]
fn push_entry_resets_scroll_when_at_last() {
    // Given a session with entries, scrolled up, cursor on last.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    // push auto-selects last (1).
    session.ui.scroll_offset = Some(0);
    assert_eq!(session.selected_entry_index(), Some(1));

    // When pushing a new entry.
    session.push_entry(ChatEntry::user("c"));

    // Then scroll is reset to bottom.
    assert!(session.is_at_bottom());
    assert_eq!(session.selected_entry_index(), Some(2));
}

#[rstest::rstest]
fn restore_history_auto_selects_last() {
    // Given history entries to restore.
    let mut session = ChatSessionState::new();

    // When restoring history with 3 entries.
    session.restore_history(vec![
        ChatEntry::user("a"),
        ChatEntry::user("b"),
        ChatEntry::user("c"),
    ]);

    // Then the last entry is auto-selected.
    assert_eq!(session.selected_entry_index(), Some(2));
    // And scroll is at bottom.
    assert!(session.is_at_bottom());
}

#[rstest::rstest]
fn restore_history_empty_clears_selection() {
    // Given a session with entries.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));

    // When restoring empty history.
    session.restore_history(vec![]);

    // Then selection is None.
    assert_eq!(session.selected_entry_index(), None);
}

#[rstest::rstest]
fn begin_thinking_auto_selects_new_last_when_at_last() {
    // Given a streaming session with cursor on user (last entry — no assistant created yet).
    let mut session = ChatSessionState::builder()
        .with_user_entry("hello")
        .begin_streaming()
        .build();
    // begin_streaming doesn't create an entry. Cursor is on user (index 0).
    assert_eq!(session.selected_entry_index(), Some(0));

    // When beginning thinking (appends Thinking at index 1).
    session.begin_thinking();

    // Then cursor advances to the new last entry (thinking at index 1).
    // history: [user(0), thinking(1)]
    assert_eq!(session.selected_entry_index(), Some(1));
    assert!(matches!(
        session.history()[1].kind,
        ChatEntryKind::Thinking(_)
    ));
}

#[rstest::rstest]
fn begin_thinking_preserves_selection_when_not_at_last() {
    // Given a streaming session with cursor NOT on assistant.
    let mut session = ChatSessionState::builder()
        .with_user_entry("hello")
        .with_user_entry("other")
        .begin_streaming()
        .build();
    // Move cursor to user entry (not the assistant).
    session.ui.selected_entry_index = Some(0);

    // When beginning thinking.
    session.begin_thinking();

    // Then cursor stays on the user entry.
    assert_eq!(session.selected_entry_index(), Some(0));
}

// --- Viewport state tests ---

#[rstest::rstest]
fn visible_entry_range_returns_visible_entries() {
    // Given a session with viewport state.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));
    session.push_entry(ChatEntry::user("d"));
    session.push_entry(ChatEntry::user("e"));

    // Entry ranges: [0..2), [2..4), [4..6), [6..8), [8..10)
    session.set_entry_line_ranges(vec![(0, 2), (2, 4), (4, 6), (6, 8), (8, 10)]);
    session.set_viewport_height(5);
    session.set_blank_count(0);
    session.ui.scroll_offset = Some(2);

    // When computing visible range.
    // viewport_top=2, viewport_bottom=7
    // Entry 1 (lines 2..4), Entry 2 (lines 4..6), Entry 3 (lines 6..8) are visible.
    let range = session.visible_entry_range();

    // Then entries 1..4 are visible.
    assert_eq!(range, 1..4);
}

#[rstest::rstest]
fn visible_entry_range_empty_when_no_ranges() {
    // Given a session with no viewport state.
    let session = ChatSessionState::new();

    // When computing visible range.
    let range = session.visible_entry_range();

    // Then it returns an empty range.
    assert!(range.is_empty());
}

#[rstest::rstest]
fn move_cursor_to_first_visible_sets_index() {
    // Given a session with viewport state.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));

    session.set_entry_line_ranges(vec![(0, 2), (2, 4), (4, 6)]);
    session.set_viewport_height(4);
    session.set_blank_count(0);
    session.ui.scroll_offset = Some(2);

    // When moving cursor to first visible.
    session.move_cursor_to_first_visible();

    // Then cursor is on the first visible entry.
    assert_eq!(session.selected_entry_index(), Some(1));
}

#[rstest::rstest]
fn move_cursor_to_last_visible_sets_index() {
    // Given a session with viewport state.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::user("b"));
    session.push_entry(ChatEntry::user("c"));

    session.set_entry_line_ranges(vec![(0, 2), (2, 4), (4, 6)]);
    session.set_viewport_height(4);
    session.set_blank_count(0);
    session.ui.scroll_offset = Some(2);

    // When moving cursor to last visible.
    session.move_cursor_to_last_visible();

    // Then cursor is on the last visible entry.
    assert_eq!(session.selected_entry_index(), Some(2));
}

// --- CWD persistence tests ---

#[rstest::rstest]
fn cwd_preserved_across_serialization_round_trip() {
    // Given a session with a specific CWD.
    let mut session = ChatSessionState::new();
    session.set_cwd(PathBuf::from("/tmp"));

    // When serializing and deserializing.
    let json = serde_json::to_string(&session).expect("serialize");
    let restored: ChatSessionState = serde_json::from_str(&json).expect("deserialize");

    // Then the CWD is preserved.
    assert_eq!(restored.cwd(), PathBuf::from("/tmp"));
}

#[rstest::rstest]
fn cwd_defaults_to_dot_when_missing_from_snapshot() {
    // Given a JSON snapshot of a session without a `cwd` field.
    let mut session = ChatSessionState::new();
    session.set_title("test".to_owned());
    let mut json = serde_json::to_value(&session).expect("serialize");
    // Remove the cwd field to simulate an old snapshot.
    json.as_object_mut().expect("object").remove("cwd");

    let json_str = serde_json::to_string(&json).expect("re-serialize");

    // When deserializing.
    let restored: ChatSessionState = serde_json::from_str(&json_str).expect("deserialize");

    // Then the CWD defaults to "." (resolves to current directory).
    assert_eq!(restored.cwd(), PathBuf::from("."));
}

#[rstest::rstest]
fn serde_round_trips_lifecycle_fields() {
    // Given a session with lifecycle fields set.
    let mut session = ChatSessionState::new();
    session.set_lifecycle_name(Some("fossil branch".to_owned()));
    session.set_lifecycle_args(vec!["feature-x".to_owned()]);

    // When serializing and deserializing.
    let json = serde_json::to_string(&session).expect("serialize");
    let back: ChatSessionState = serde_json::from_str(&json).expect("deserialize");

    // Then lifecycle fields are preserved.
    assert_eq!(back.lifecycle_name(), Some("fossil branch"));
    assert_eq!(back.lifecycle_args(), &["feature-x".to_owned()]);
}

#[rstest::rstest]
fn serde_defaults_lifecycle_fields_when_missing() {
    // Given a JSON object without lifecycle fields.
    let json = r#"{"session_id":"test","updated_at":"2026-01-01T00:00:00Z","created_at":"2026-01-01T00:00:00Z","history":[],"profile":{"model":"","strategy":"passthrough"},"cwd":"."}"#;

    // When deserializing.
    let back: ChatSessionState = serde_json::from_str(json).expect("deserialize");

    // Then lifecycle fields default to None/empty.
    assert!(back.lifecycle_name().is_none());
    assert!(back.lifecycle_args().is_empty());
}
// --- is_empty tests ---

#[rstest::rstest]
fn is_empty_true_for_new_session() {
    // Given a newly created session.
    let session = ChatSessionState::new();

    // Then it is empty.
    assert!(session.is_empty());
}

#[rstest::rstest]
fn is_empty_false_after_pushing_entry() {
    // Given a new session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // Then it is not empty.
    assert!(!session.is_empty());
}

// --- Streaming tool result tests ---

#[test]
fn begin_tool_result_creates_pending_entry() {
    // Given a default session.
    let mut session = ChatSessionState::new();

    // When beginning a tool result.
    session.begin_tool_result("call_1", "bash");

    // Then the history has a pending ToolResult entry.
    assert_eq!(session.history().len(), 1);
    let entry = &session.history()[0];
    match &entry.kind {
        ChatEntryKind::ToolResult {
            id,
            name,
            content,
            status,
        } => {
            assert_eq!(id, "call_1");
            assert_eq!(name, "bash");
            assert!(content.is_empty());
            assert_eq!(*status, ToolResultStatus::Pending);
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
}

#[test]
fn begin_tool_result_tracks_history_index() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // When beginning a tool result.
    session.begin_tool_result("call_1", "bash");

    // Then the tracking index points to the second entry.
    assert!(session
        .core
        .ephemeral
        .streaming_tool_result_indices
        .contains_key("call_1"));
    assert_eq!(session.core.ephemeral.streaming_tool_result_indices["call_1"], 1);
}

#[test]
fn append_tool_result_output_appends_to_pending_entry() {
    // Given a session with a pending tool result.
    let mut session = ChatSessionState::new();
    session.begin_tool_result("call_1", "bash");

    // When appending output.
    session.append_tool_result_output("call_1", "line 1
");
    session.append_tool_result_output("call_1", "line 2
");

    // Then the entry content has both outputs.
    match &session.history()[0].kind {
        ChatEntryKind::ToolResult { content, .. } => {
            assert_eq!(content, "line 1
line 2
");
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
}

#[test]
fn append_tool_result_output_ignores_unknown_call_id() {
    // Given a session with no pending tool result.
    let mut session = ChatSessionState::new();

    // When appending output for an unknown call ID.
    // Then it does not panic (defensive).
    session.append_tool_result_output("unknown", "output");
    assert!(session.history().is_empty());
}

#[test]
fn finalize_tool_result_completes_pending_entry() {
    // Given a session with a pending tool result.
    let mut session = ChatSessionState::new();
    session.begin_tool_result("call_1", "bash");
    session.append_tool_result_output("call_1", "building...
");

    // When finalizing with success.
    session.finalize_tool_result("call_1", "bash", "final output", true);

    // Then the entry is updated with final content and Success status.
    assert_eq!(session.history().len(), 1);
    match &session.history()[0].kind {
        ChatEntryKind::ToolResult {
            content,
            status,
            ..
        } => {
            assert_eq!(content, "final output");
            assert_eq!(*status, ToolResultStatus::Success);
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
    // And the tracking index is removed.
    assert!(!session
        .core
        .ephemeral
        .streaming_tool_result_indices
        .contains_key("call_1"));
}

#[test]
fn finalize_tool_result_pushes_new_entry_for_unknown_id() {
    // Given a session with no pending tool result.
    let mut session = ChatSessionState::new();

    // When finalizing for a tool that never streamed.
    session.finalize_tool_result("call_1", "bash", "output", true);

    // Then a new entry is pushed.
    assert_eq!(session.history().len(), 1);
    match &session.history()[0].kind {
        ChatEntryKind::ToolResult {
            id,
            name,
            content,
            status,
        } => {
            assert_eq!(id, "call_1");
            assert_eq!(name, "bash");
            assert_eq!(content, "output");
            assert_eq!(*status, ToolResultStatus::Success);
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
}
