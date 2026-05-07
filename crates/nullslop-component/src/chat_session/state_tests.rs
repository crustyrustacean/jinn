use nullslop_protocol::ChatEntry;

use super::*;

#[test]
fn push_entry_adds_to_history() {
    // Given a new ChatSessionState.
    let mut session = ChatSessionState::new();

    // When pushing a user entry.
    let index = session.push_entry(ChatEntry::user("hello"));

    // Then the index is 0 and history has one entry.
    assert_eq!(index, 0);
    assert_eq!(session.history().len(), 1);
}

#[test]
fn begin_streaming_creates_assistant_entry_and_sets_streaming() {
    // Given a session with one entry.
    let mut session = ChatSessionState::new();
    session.push_entry(ChatEntry::user("hello"));

    // When beginning streaming.
    let index = session.begin_streaming();

    // Then the index is 1, is_streaming is true, and history has an Assistant entry.
    assert_eq!(index, 1);
    assert!(session.is_streaming());
    assert_eq!(session.history().len(), 2);
    assert!(matches!(
        session.history()[1].kind,
        nullslop_protocol::ChatEntryKind::Assistant(ref text) if text.is_empty()
    ));
}

#[test]
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
        nullslop_protocol::ChatEntryKind::Assistant("Hello world".to_owned())
    );
}

#[test]
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
        nullslop_protocol::ChatEntryKind::Assistant("Hi".to_owned())
    );
}

#[test]
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
        nullslop_protocol::ChatEntryKind::Assistant("Partial".to_owned())
    );
}

#[test]
#[should_panic(expected = "begin_streaming called while already streaming")]
fn begin_streaming_twice_panics() {
    // Given a session that is already streaming.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When calling begin_streaming again.
    // Then it panics.
    session.begin_streaming();
}

#[test]
#[should_panic(expected = "append_stream_token called while not streaming")]
fn append_stream_token_when_not_streaming_panics() {
    // Given a session that is not streaming.
    let mut session = ChatSessionState::new();

    // When calling append_stream_token.
    // Then it panics.
    session.append_stream_token("oops");
}

#[test]
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

#[test]
fn scroll_up_from_known_offset_decrements() {
    // Given a session with scroll_offset = 50 and last_max_offset = 100.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.scroll_offset = Some(50);

    // When scrolling up by 10.
    session.scroll_up(10);

    // Then the offset is 40.
    assert_eq!(session.scroll_offset(), Some(40));
}

#[test]
fn scroll_up_saturates_at_zero() {
    // Given a session with scroll_offset = 5 and last_max_offset = 100.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.scroll_offset = Some(5);

    // When scrolling up by 20.
    session.scroll_up(20);

    // Then the offset saturates at 0.
    assert_eq!(session.scroll_offset(), Some(0));
}

#[test]
fn scroll_down_increments_offset() {
    // Given a session with scroll_offset = 0 and last_max_offset = 100.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.scroll_offset = Some(0);

    // When scrolling down by 10.
    session.scroll_down(10);

    // Then the offset increased by 10.
    assert_eq!(session.scroll_offset(), Some(10));
}

#[test]
fn scroll_down_past_bottom_resets_to_auto() {
    // Given a session with scroll_offset = 95 and last_max_offset = 100.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.scroll_offset = Some(95);

    // When scrolling down by 10.
    session.scroll_down(10);

    // Then the offset resets to None (auto-scroll to bottom).
    assert!(session.scroll_offset().is_none());
}

#[test]
fn scroll_to_top_sets_offset_to_zero() {
    // Given a session scrolled to the middle.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.scroll_offset = Some(50);

    // When scrolling to top.
    session.scroll_to_top();

    // Then the offset is 0.
    assert_eq!(session.scroll_offset(), Some(0));
}

#[test]
fn scroll_to_bottom_resets_to_auto_scroll() {
    // Given a session scrolled to the top.
    let mut session = ChatSessionState::new();
    session.set_last_max_offset(100);
    session.scroll_offset = Some(0);

    // When scrolling to bottom.
    session.scroll_to_bottom();

    // Then the offset is None (auto-scroll).
    assert!(session.scroll_offset().is_none());
}

#[test]
fn reset_scroll_clears_offset() {
    // Given a session with scroll_offset = 50.
    let mut session = ChatSessionState::new();
    session.scroll_offset = Some(50);

    // When resetting scroll.
    session.reset_scroll();

    // Then the offset is None (at bottom).
    assert!(session.scroll_offset().is_none());
}

#[test]
fn push_entry_resets_scroll() {
    // Given a session with scroll_offset = 50.
    let mut session = ChatSessionState::new();
    session.scroll_offset = Some(50);

    // When pushing an entry.
    session.push_entry(ChatEntry::user("hello"));

    // Then scroll_offset is None (reset by push_entry).
    assert!(session.scroll_offset().is_none());
}

#[test]
fn is_at_bottom_true_when_auto_scroll() {
    // Given a new session (auto-scroll to bottom).
    let session = ChatSessionState::new();

    // Then is_at_bottom is true.
    assert!(session.is_at_bottom());
}

#[test]
fn is_at_bottom_false_when_scrolled_up() {
    // Given a session scrolled to offset 50.
    let mut session = ChatSessionState::new();
    session.scroll_offset = Some(50);

    // Then is_at_bottom is false.
    assert!(!session.is_at_bottom());
}

// --- Queue tests ---

#[test]
fn enqueue_message_adds_to_queue() {
    // Given a new session with an empty queue.
    let mut session = ChatSessionState::new();
    assert_eq!(session.queue_len(), 0);

    // When enqueuing a message.
    session.enqueue_message("hello".to_owned());

    // Then the queue has one message.
    assert_eq!(session.queue_len(), 1);
    assert_eq!(session.queue()[0], "hello");
}

#[test]
fn dequeue_message_returns_first_in_order() {
    // Given a session with two queued messages.
    let mut session = ChatSessionState::new();
    session.enqueue_message("first".to_owned());
    session.enqueue_message("second".to_owned());

    // When dequeuing a message.
    let msg = session.dequeue_message();

    // Then it returns the first message and the queue has one left.
    assert_eq!(msg.as_deref(), Some("first"));
    assert_eq!(session.queue_len(), 1);
}

#[test]
fn dequeue_message_returns_none_when_empty() {
    // Given a session with an empty queue.
    let mut session = ChatSessionState::new();

    // When dequeuing a message.
    let msg = session.dequeue_message();

    // Then it returns None.
    assert!(msg.is_none());
}

#[test]
fn drain_queue_empties_and_returns_all() {
    // Given a session with three queued messages.
    let mut session = ChatSessionState::new();
    session.enqueue_message("a".to_owned());
    session.enqueue_message("b".to_owned());
    session.enqueue_message("c".to_owned());

    // When draining the queue.
    let drained = session.drain_queue();

    // Then all messages are returned in order and the queue is empty.
    assert_eq!(drained.len(), 3);
    assert_eq!(drained[0], "a");
    assert_eq!(drained[1], "b");
    assert_eq!(drained[2], "c");
    assert_eq!(session.queue_len(), 0);
}

// --- Sending tests ---

#[test]
fn begin_sending_sets_is_sending() {
    // Given a new session (idle).
    let mut session = ChatSessionState::new();
    assert!(!session.is_sending());

    // When beginning sending.
    session.begin_sending();

    // Then is_sending is true.
    assert!(session.is_sending());
}

#[test]
#[should_panic(expected = "begin_sending called while already sending or streaming")]
fn begin_sending_panics_when_already_sending() {
    // Given a session that is already sending.
    let mut session = ChatSessionState::new();
    session.begin_sending();

    // When calling begin_sending again.
    // Then it panics.
    session.begin_sending();
}

#[test]
#[should_panic(expected = "begin_sending called while already sending or streaming")]
fn begin_sending_panics_when_streaming() {
    // Given a session that is streaming.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When calling begin_sending.
    // Then it panics.
    session.begin_sending();
}

#[test]
fn finish_sending_clears_flag() {
    // Given a session that is sending.
    let mut session = ChatSessionState::new();
    session.begin_sending();

    // When finishing sending.
    session.finish_sending();

    // Then is_sending is false.
    assert!(!session.is_sending());
}

#[test]
#[should_panic(expected = "finish_sending called while not sending")]
fn finish_sending_panics_when_not_sending() {
    // Given a session that is not sending.
    let mut session = ChatSessionState::new();

    // When calling finish_sending.
    // Then it panics.
    session.finish_sending();
}

// --- Combined status tests ---

#[test]
fn is_idle_true_when_not_sending_or_streaming() {
    // Given a fresh session.
    let session = ChatSessionState::new();

    // Then it is idle.
    assert!(session.is_idle());
}

#[test]
fn is_idle_false_when_sending() {
    // Given a session that is sending.
    let mut session = ChatSessionState::new();
    session.begin_sending();

    // Then it is not idle.
    assert!(!session.is_idle());
}

#[test]
fn is_idle_false_when_streaming() {
    // Given a session that is streaming.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // Then it is not idle.
    assert!(!session.is_idle());
}

#[test]
fn cancel_streaming_clears_sending_too() {
    // Given a session that was sending before streaming started.
    let mut session = ChatSessionState::new();
    session.begin_sending();
    // Simulate: stream started (sending still set until first token clears it).
    // We need to manipulate internals since normally begin_streaming would panic
    // when is_sending is true. So we manually set is_streaming.
    session.is_streaming = true;
    assert!(session.is_sending());
    assert!(session.is_streaming());

    // When cancelling streaming.
    session.cancel_streaming();

    // Then both flags are cleared.
    assert!(!session.is_sending());
    assert!(!session.is_streaming());
}

#[test]
fn finish_streaming_clears_sending_too() {
    // Given a session that was sending before streaming started.
    let mut session = ChatSessionState::new();
    session.begin_sending();
    // Manually set is_streaming to simulate the transition.
    session.is_streaming = true;
    session.streaming_entry_index = Some(session.push_entry(ChatEntry::assistant("")));

    // When finishing streaming.
    session.finish_streaming();

    // Then both flags are cleared.
    assert!(!session.is_sending());
    assert!(!session.is_streaming());
}

// --- Tool call streaming tests ---

#[test]
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

#[test]
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

#[test]
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

#[test]
fn finalize_tool_call_pushes_new_entry_when_not_found() {
    // Given a streaming session with no tool call entry for the given ID.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When finalizing a tool call that was never started (shouldn't happen normally).
    session.finalize_tool_call("call_99", "echo", r#"{"input":"hi"}"#);

    // Then a new entry is pushed to history.
    assert_eq!(session.history().len(), 2); // assistant + new tool call
    assert_eq!(
        session.history()[1].kind,
        ChatEntryKind::ToolCall {
            id: "call_99".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        }
    );
}

#[test]
fn multiple_tool_calls_track_independently() {
    // Given a streaming session.
    let mut session = ChatSessionState::new();
    session.begin_streaming();

    // When beginning two tool calls with different indices.
    session.begin_tool_call(0, "call_1", "echo");
    session.append_tool_call_delta(0, r#"{"a":1}"#);

    session.begin_tool_call(1, "call_2", "get_time");
    session.append_tool_call_delta(1, "{}");

    // Then each entry tracks its own arguments.
    assert_eq!(
        session.history()[1].kind,
        ChatEntryKind::ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"a":1}"#.to_owned(),
        }
    );
    assert_eq!(
        session.history()[2].kind,
        ChatEntryKind::ToolCall {
            id: "call_2".to_owned(),
            name: "get_time".to_owned(),
            arguments: "{}".to_owned(),
        }
    );
}

#[test]
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

#[test]
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

#[test]
fn default_strategy_is_passthrough() {
    // Given a new session.
    let session = ChatSessionState::new();

    // Then the default strategy is passthrough.
    assert_eq!(
        session.active_strategy(),
        &nullslop_protocol::PromptStrategyId::passthrough()
    );
}

#[test]
fn switch_strategy_updates_active_strategy() {
    // Given a new session.
    let mut session = ChatSessionState::new();

    // When switching to sliding_window.
    session.switch_strategy(nullslop_protocol::PromptStrategyId::sliding_window());

    // Then the active strategy is updated.
    assert_eq!(
        session.active_strategy(),
        &nullslop_protocol::PromptStrategyId::sliding_window()
    );
}

#[test]
fn new_with_strategy_sets_active_strategy() {
    // Given a strategy ID.
    let strategy = nullslop_protocol::PromptStrategyId::sliding_window();

    // When creating a session with that strategy.
    let session = ChatSessionState::new_with_strategy(strategy.clone());

    // Then the active strategy is set to the given strategy.
    assert_eq!(session.active_strategy(), &strategy);
}

#[test]
fn new_with_strategy_creates_empty_history() {
    // Given any strategy.
    let strategy = nullslop_protocol::PromptStrategyId::compaction();

    // When creating a session with that strategy.
    let session = ChatSessionState::new_with_strategy(strategy);

    // Then the history is empty.
    assert!(session.history().is_empty());
}
