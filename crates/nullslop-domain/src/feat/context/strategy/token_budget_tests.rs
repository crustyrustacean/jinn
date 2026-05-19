#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::feat::context::strategy::token_budget::TokenBudgetStrategy;
use crate::feat::context::strategy::token_estimator::CharRatioEstimator;
use crate::feat::context::strategy::types::{AssemblyContext, PromptAssembly};
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::{ChatEntry, LlmMessage, PinPosition, SessionId};

fn test_context<'a>(history: &'a [ChatEntry], session_id: &'a SessionId) -> AssemblyContext<'a> {
    AssemblyContext {
        history,
        tools: &[],
        model_name: "test-model",
        session_id,
        budget_offset: 0,
    }
}

fn test_context_with_overhead<'a>(
    history: &'a [ChatEntry],
    session_id: &'a SessionId,
    budget_offset: usize,
) -> AssemblyContext<'a> {
    AssemblyContext {
        history,
        tools: &[],
        model_name: "test-model",
        session_id,
        budget_offset,
    }
}

fn make_strategy(max_tokens: usize) -> TokenBudgetStrategy {
    TokenBudgetStrategy::new(max_tokens, Box::new(CharRatioEstimator))
}

#[rstest::rstest]
#[tokio::test]
async fn truncates_history_to_fit_budget() {
    // Given 5 entries with ~100-char content each (~26 tokens each, ~130 total)
    // and a budget of 80 tokens.
    let history: Vec<ChatEntry> = (0..5)
        .map(|i| {
            let mut s = "a".repeat(100);
            s.push_str(&i.to_string());
            ChatEntry::user(s)
        })
        .collect();
    let strategy = make_strategy(80);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then fewer than 5 entries are included and system prompt is set.
    assert!(result.messages.len() < 5);
    assert!(result.system_prompt.is_some());
}

#[rstest::rstest]
#[tokio::test]
async fn returns_all_entries_when_under_budget() {
    // Given 3 short entries that easily fit in a large budget.
    let history = vec![
        ChatEntry::user("hi"),
        ChatEntry::assistant("hello"),
        ChatEntry::user("how are you?"),
    ];
    let strategy = make_strategy(8192);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then all entries are included with no system prompt.
    assert_eq!(result.messages.len(), 3);
    assert!(result.system_prompt.is_none());
}

#[rstest::rstest]
#[tokio::test]
async fn empty_history_produces_no_messages() {
    // Given empty history.
    let history: Vec<ChatEntry> = vec![];
    let strategy = make_strategy(8192);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then no messages are produced.
    assert!(result.messages.is_empty());
    assert!(result.system_prompt.is_none());
}

#[rstest::rstest]
#[tokio::test]
async fn single_over_budget_entry_is_included_anyway() {
    // Given one entry that far exceeds the budget.
    let history = vec![ChatEntry::user("x".repeat(1000))];
    let strategy = make_strategy(10);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then the entry is still included (no trimming occurred on a single entry).
    assert_eq!(result.messages.len(), 1);
    assert!(result.system_prompt.is_none());
}

#[rstest::rstest]
#[tokio::test]
async fn system_prompt_set_when_trimmed() {
    // Given entries that exceed the budget.
    let history = vec![
        ChatEntry::user("a".repeat(200)),
        ChatEntry::assistant("b".repeat(200)),
        ChatEntry::user("short"),
    ];
    let strategy = make_strategy(30);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then system prompt is set indicating context was trimmed.
    assert!(result.system_prompt.is_some());
    assert_eq!(
        result.system_prompt.as_deref(),
        Some("Some earlier context was omitted to fit within the token budget.")
    );
}

#[rstest::rstest]
#[tokio::test]
async fn no_system_prompt_when_nothing_trimmed() {
    // Given entries that fit within the budget.
    let history = vec![ChatEntry::user("hi"), ChatEntry::assistant("hello")];
    let strategy = make_strategy(8192);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then no system prompt is set.
    assert!(result.system_prompt.is_none());
}

#[rstest::rstest]
#[tokio::test]
async fn preserves_chronological_order() {
    // Given 3 entries where the first exceeds the budget when combined.
    let history = vec![
        ChatEntry::user("a".repeat(200)),
        ChatEntry::assistant("b".repeat(200)),
        ChatEntry::user("short"),
    ];
    let strategy = make_strategy(60);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then the included messages maintain chronological order.
    assert!(!result.messages.is_empty());
    // The last message should be the most recent ("short" user message).
    let last = result.messages.last().expect("should have messages");
    assert_eq!(
        last,
        &LlmMessage::User {
            content: "short".to_owned(),
        }
    );
}

#[rstest::rstest]
#[tokio::test]
async fn name_returns_token_budget() {
    // Given a token budget strategy.
    let strategy = make_strategy(8192);

    // Then its name is "token_budget".
    assert_eq!(strategy.name(), "token_budget");
}

#[rstest::rstest]
#[tokio::test]
async fn newest_entry_included_when_rest_trimmed() {
    // Given many entries where only the newest fits.
    let mut history = Vec::new();
    for _ in 0..10 {
        history.push(ChatEntry::user("x".repeat(100)));
    }
    // Most recent is short.
    history.push(ChatEntry::user("ok"));
    let strategy = make_strategy(10);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then at least the most recent entry is included.
    assert!(!result.messages.is_empty());
    let last = result.messages.last().expect("should have messages");
    assert_eq!(
        last,
        &LlmMessage::User {
            content: "ok".to_owned(),
        }
    );
}

#[rstest::rstest]
#[tokio::test]
async fn pinned_entry_survives_token_budget_trimming() {
    // Given 4 entries where the first (oldest) is pinned, and a tight budget.
    let history = vec![
        ChatEntry::user("pinned").with_pin(PinPosition::Top),
        ChatEntry::user("a".repeat(100)),
        ChatEntry::user("b".repeat(100)),
        ChatEntry::user("recent"),
    ];
    // Budget fits recent + pinned but not the middle entries.
    let strategy = make_strategy(10);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then the pinned entry and the most recent entry are included.
    let contents: Vec<&str> = result
        .messages
        .iter()
        .map(|m| match m {
            LlmMessage::User { content } => content.as_str(),
            other => panic!("expected User, got {other:?}"),
        })
        .collect();
    assert!(contents.contains(&"pinned"));
    assert!(contents.contains(&"recent"));
}

#[rstest::rstest]
#[tokio::test]
async fn pinned_entry_tokens_count_toward_budget() {
    // Given entries with a pinned entry in the middle, and a tight budget.
    let history = vec![
        ChatEntry::user("old"),
        ChatEntry::user("x".repeat(100)).with_pin(PinPosition::Relative),
        ChatEntry::user("mid"),
        ChatEntry::user("recent"),
    ];
    // Budget: "recent"(~2) + "mid"(~1) + pinned"x"*100(~26) = ~29 tokens.
    // "old"(~1) would push to ~30, but budget is 28 so it's excluded.
    let strategy = make_strategy(28);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then the pinned entry consumes budget, crowding out the oldest entry.
    assert!(result.messages.len() < 4);
    let contents: Vec<&str> = result
        .messages
        .iter()
        .map(|m| match m {
            LlmMessage::User { content } => content.as_str(),
            other => panic!("expected User, got {other:?}"),
        })
        .collect();
    assert!(contents.contains(&"x".repeat(100).as_str()));
    assert!(!contents.contains(&"old"));
}

#[rstest::rstest]
#[tokio::test]
async fn multiple_pinned_entries_survive_budget_trimming() {
    // Given 5 entries where entry 0 and entry 2 are pinned.
    let history = vec![
        ChatEntry::user("pinned-early").with_pin(PinPosition::Top),
        ChatEntry::user("a".repeat(100)),
        ChatEntry::user("pinned-mid").with_pin(PinPosition::Relative),
        ChatEntry::user("b".repeat(100)),
        ChatEntry::user("recent"),
    ];
    let strategy = make_strategy(30);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then both pinned entries survive regardless of budget.
    let contents: Vec<&str> = result
        .messages
        .iter()
        .map(|m| match m {
            LlmMessage::User { content } => content.as_str(),
            other => panic!("expected User, got {other:?}"),
        })
        .collect();
    assert!(contents.contains(&"pinned-early"));
    assert!(contents.contains(&"pinned-mid"));
}

#[rstest::rstest]
#[tokio::test]
async fn pinned_entry_does_not_prevent_newest_entry() {
    // Given a pinned entry and a most recent entry, both exceeding budget.
    let history = vec![
        ChatEntry::user("pinned".repeat(50)).with_pin(PinPosition::Relative),
        ChatEntry::user("ok"),
    ];
    let strategy = make_strategy(5);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then both pinned and most recent are included.
    assert_eq!(result.messages.len(), 2);
    let last = result.messages.last().expect("should have messages");
    assert_eq!(
        last,
        &LlmMessage::User {
            content: "ok".to_owned(),
        }
    );
}

#[rstest::rstest]
#[tokio::test]
async fn budget_offset_reduces_effective_history_budget() {
    // Given 4 short entries that easily fit in budget with offset=0.
    let history = vec![
        ChatEntry::user("first"),
        ChatEntry::assistant("second"),
        ChatEntry::user("third"),
        ChatEntry::user("fourth"),
    ];
    let session_id = SessionId::new();

    // When assembling with no offset, all entries fit.
    let strategy = make_strategy(100);
    let context_no_offset = test_context(&history, &session_id);
    let result_no_offset = strategy
        .assemble(&context_no_offset)
        .await
        .expect("assemble");
    assert_eq!(result_no_offset.messages.len(), 4);

    // When assembling with a large offset, entries are trimmed.
    let context_with_offset = test_context_with_overhead(&history, &session_id, 95);
    let result_with_offset = strategy
        .assemble(&context_with_offset)
        .await
        .expect("assemble");

    // Then fewer entries survive because the effective budget is only 5 tokens.
    assert!(result_with_offset.messages.len() < 4);
}

#[rstest::rstest]
#[tokio::test]
async fn budget_offset_equal_to_max_tokens_leaves_only_newest() {
    // Given 3 entries where the newest is very short.
    let history = vec![
        ChatEntry::user("a fairly long first message"),
        ChatEntry::assistant("a fairly long response"),
        ChatEntry::user("ok"),
    ];
    // Budget is 100, offset is 100, so effective budget is 0.
    let strategy = make_strategy(100);
    let session_id = SessionId::new();
    let context = test_context_with_overhead(&history, &session_id, 100);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then only the most recent entry survives (at-least-one guarantee).
    assert!(!result.messages.is_empty());
    let last = result.messages.last().expect("should have messages");
    assert_eq!(
        last,
        &LlmMessage::User {
            content: "ok".to_owned(),
        }
    );
}

#[rstest::rstest]
#[tokio::test]
async fn tool_loop_turn_never_split_by_budget() {
    // Given a large user entry followed by a tool-loop turn, with a tight budget.
    let history = vec![
        ChatEntry::user("a".repeat(2000)),
        ChatEntry::assistant("let me check"),
        ChatEntry::tool_call("call_1", "echo", "{}"),
        ChatEntry::tool_result("call_1", "echo", "ok", ToolResultStatus::Success),
    ];
    let strategy = make_strategy(100);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then either all tool-loop entries are included or all are excluded.
    let messages = &result.messages;
    let has_tool_calls = messages.iter().any(|m| {
        matches!(
            m,
            LlmMessage::Assistant {
                tool_calls: Some(_),
                ..
            }
        )
    });
    let has_tool_result = messages
        .iter()
        .any(|m| matches!(m, LlmMessage::Tool { .. }));
    // Both or neither, never one without the other.
    assert_eq!(has_tool_calls, has_tool_result);
}

#[rstest::rstest]
#[tokio::test]
async fn pinned_entry_inside_tool_loop_forces_entire_turn() {
    // Given a tool-loop turn where the ToolResult is pinned, with a tight budget.
    let history = vec![
        ChatEntry::assistant("small check"),
        ChatEntry::tool_call("call_1", "echo", "{}"),
        ChatEntry::tool_result("call_1", "echo", "ok", ToolResultStatus::Success)
            .with_pin(PinPosition::Relative),
    ];
    let strategy = make_strategy(50);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then all three entries are included (the pinned ToolResult forces
    // the entire tool-loop turn to be included).
    assert!(result.messages.iter().any(|m| {
        matches!(
            m,
            LlmMessage::Assistant {
                tool_calls: Some(_),
                ..
            }
        )
    }));
    assert!(
        result
            .messages
            .iter()
            .any(|m| matches!(m, LlmMessage::Tool { .. }))
    );
}
