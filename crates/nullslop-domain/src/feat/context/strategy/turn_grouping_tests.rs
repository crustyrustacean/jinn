#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::feat::context::strategy::token_estimator::CharRatioEstimator;
use crate::feat::context::strategy::turn_grouping::{Turn, group_into_turns};
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::{ChatEntry, PinPosition};

fn turn_entry_counts(turns: &[Turn]) -> Vec<usize> {
    turns.iter().map(|t| t.entry_count()).collect()
}

fn turn_kinds(turns: &[Turn]) -> Vec<Vec<&'static str>> {
    turns
        .iter()
        .map(|t| t.entries().map(|e| e.kind_str()).collect())
        .collect()
}

#[test]
fn empty_history_produces_no_turns() {
    // Given no history.
    let history: Vec<ChatEntry> = vec![];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then no turns are produced.
    assert!(turns.is_empty());
}

#[test]
fn user_entry_is_standalone_turn() {
    // Given a single user entry.
    let history = vec![ChatEntry::user("hi")];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then one turn with one entry is produced.
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].entry_count(), 1);
}

#[test]
fn assistant_without_tool_call_is_standalone() {
    // Given a single assistant entry.
    let history = vec![ChatEntry::assistant("hi")];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then one turn with one entry is produced.
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].entry_count(), 1);
}

#[test]
fn assistant_tool_call_tool_result_is_one_turn() {
    // Given an assistant followed by a tool call and tool result.
    let history = vec![
        ChatEntry::assistant("let me check"),
        ChatEntry::tool_call("call_1", "echo", r#"{"input":"hi"}"#),
        ChatEntry::tool_result("call_1", "echo", "hi", ToolResultStatus::Success),
    ];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then one turn with three entries is produced.
    assert_eq!(turns.len(), 1);
    assert_eq!(turn_entry_counts(&turns), [3]);
}

#[test]
fn multiple_tool_calls_after_one_assistant() {
    // Given an assistant followed by two parallel tool calls and their results.
    let history = vec![
        ChatEntry::assistant("checking both"),
        ChatEntry::tool_call("call_1", "echo", r#"{"input":"a"}"#),
        ChatEntry::tool_call("call_2", "get_time", "{}"),
        ChatEntry::tool_result("call_1", "echo", "a", ToolResultStatus::Success),
        ChatEntry::tool_result("call_2", "get_time", "12:00", ToolResultStatus::Success),
    ];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then one turn with five entries is produced.
    assert_eq!(turns.len(), 1);
    assert_eq!(turn_entry_counts(&turns), [5]);
}

#[test]
fn multi_round_tool_loop_produces_separate_turns() {
    // Given two back-to-back tool loops.
    let history = vec![
        ChatEntry::assistant("round 1"),
        ChatEntry::tool_call("call_1", "echo", "{}"),
        ChatEntry::tool_result("call_1", "echo", "ok", ToolResultStatus::Success),
        ChatEntry::assistant("round 2"),
        ChatEntry::tool_call("call_2", "bash", "{}"),
        ChatEntry::tool_result("call_2", "bash", "done", ToolResultStatus::Success),
    ];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then two tool-loop turns are produced, each with three entries.
    assert_eq!(turns.len(), 2);
    assert_eq!(turn_entry_counts(&turns), [3, 3]);
}

#[test]
fn orphaned_tool_call_is_standalone_turn() {
    // Given a tool call without a preceding assistant.
    let history = vec![ChatEntry::tool_call("call_1", "echo", "{}")];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then one standalone turn with one entry is produced.
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].entry_count(), 1);
}

#[test]
fn orphaned_tool_result_is_standalone_turn() {
    // Given a tool result without a preceding tool call.
    let history = vec![ChatEntry::tool_result(
        "call_1",
        "echo",
        "hi",
        ToolResultStatus::Success,
    )];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then one standalone turn with one entry is produced.
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].entry_count(), 1);
}

#[test]
fn mixed_user_and_tool_loop_entries() {
    // Given a user entry, tool loop, and another user entry.
    let history = vec![
        ChatEntry::user("go"),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("call_1", "echo", "{}"),
        ChatEntry::tool_result("call_1", "echo", "ok", ToolResultStatus::Success),
        ChatEntry::user("thanks"),
    ];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then three turns: [User], [Assistant+ToolCall+ToolResult], [User].
    assert_eq!(turns.len(), 3);
    assert_eq!(turn_entry_counts(&turns), [1, 3, 1]);
}

#[test]
fn turn_token_cost_sums_entry_tokens() {
    // Given a turn with two user entries.
    let history = vec![ChatEntry::user("hello"), ChatEntry::user("world")];
    let turns = group_into_turns(&history);
    assert_eq!(turns.len(), 2);

    // When computing token cost for the first turn.
    let estimator = CharRatioEstimator;
    let cost = turns[0].token_cost(&estimator);

    // Then the cost equals the estimate for "hello" alone (standalone turn).
    let expected = crate::feat::context::strategy::token_estimator::estimate_entry_tokens(
        &estimator,
        &history[0],
    );
    assert_eq!(cost, expected);
}

#[test]
fn turn_is_pinned_reflects_any_pinned_entry() {
    // Given a tool-loop turn where only the ToolResult is pinned.
    let history = vec![
        ChatEntry::assistant("check"),
        ChatEntry::tool_call("call_1", "echo", "{}"),
        ChatEntry::tool_result("call_1", "echo", "ok", ToolResultStatus::Success)
            .with_pin(PinPosition::Relative),
    ];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then the single turn is marked as pinned.
    assert_eq!(turns.len(), 1);
    assert!(turns[0].is_pinned());
}

#[test]
fn system_entry_is_standalone_turn() {
    // Given a system entry.
    let history = vec![ChatEntry::system("welcome")];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then one standalone turn with one entry is produced.
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].entry_count(), 1);
}

#[test]
fn full_conversation_produces_correct_turns() {
    // Given a full conversation with system, user, tool loop, and final assistant.
    let history = vec![
        ChatEntry::system("you are helpful"),
        ChatEntry::user("what time is it?"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("call_1", "get_time", "{}"),
        ChatEntry::tool_result("call_1", "get_time", "12:00", ToolResultStatus::Success),
        ChatEntry::assistant("It's 12:00!"),
    ];

    // When grouping into turns.
    let turns = group_into_turns(&history);

    // Then four turns: [System], [User], [Assistant+ToolCall+ToolResult], [Assistant].
    assert_eq!(turns.len(), 4);
    assert_eq!(turn_entry_counts(&turns), [1, 1, 3, 1]);
    let kinds = turn_kinds(&turns);
    assert_eq!(kinds[0], vec!["system"]);
    assert_eq!(kinds[1], vec!["user"]);
    assert_eq!(kinds[2], vec!["assistant", "tool_call", "tool_result"]);
    assert_eq!(kinds[3], vec!["assistant"]);
}
