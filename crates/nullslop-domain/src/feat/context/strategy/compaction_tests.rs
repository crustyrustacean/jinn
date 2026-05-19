#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::feat::context::strategy::compaction::CompactionStrategy;
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

fn make_strategy(max_tokens: usize) -> CompactionStrategy {
    CompactionStrategy::new(max_tokens, Box::new(CharRatioEstimator))
}

#[rstest::rstest]
#[tokio::test]
async fn returns_all_entries_when_under_threshold() {
    // Given entries that fit within the threshold.
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
async fn trims_entries_when_over_threshold() {
    // Given entries that exceed the threshold.
    let history: Vec<ChatEntry> = std::iter::repeat_with(|| ChatEntry::user("a".repeat(400)))
        .take(10)
        .collect();
    let strategy = make_strategy(100);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then fewer entries are included with a compaction system prompt.
    assert!(result.messages.len() < 10);
    assert!(result.system_prompt.is_some());
    assert_eq!(
        result.system_prompt.as_deref(),
        Some(
            "Context was compacted to fit within the token budget. Earlier conversation history was summarized."
        )
    );
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
async fn single_over_threshold_entry_is_included_anyway() {
    // Given one entry that far exceeds the threshold.
    let history = vec![ChatEntry::user("x".repeat(1000))];
    let strategy = make_strategy(10);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then the entry is still included (over threshold triggers trimming).
    assert_eq!(result.messages.len(), 1);
    // System prompt is set because we're over threshold.
    assert!(result.system_prompt.is_some());
}

#[rstest::rstest]
#[tokio::test]
async fn compaction_system_prompt_differs_from_token_budget() {
    // Given entries that trigger compaction.
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

    // Then the compaction system prompt is distinct from token budget's.
    assert_ne!(
        result.system_prompt.as_deref(),
        Some("Some earlier context was omitted to fit within the token budget.")
    );
    assert_eq!(
        result.system_prompt.as_deref(),
        Some(
            "Context was compacted to fit within the token budget. Earlier conversation history was summarized."
        )
    );
}

#[rstest::rstest]
#[tokio::test]
async fn preserves_chronological_order() {
    // Given 3 entries where trimming occurs.
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
async fn name_returns_compaction() {
    // Given a compaction strategy.
    let strategy = make_strategy(8192);

    // Then its name is "compaction".
    assert_eq!(strategy.name(), "compaction");
}

#[rstest::rstest]
#[tokio::test]
async fn pinned_entry_survives_compaction_trimming() {
    // Given 4 entries where the first (oldest) is pinned, over threshold.
    let history = vec![
        ChatEntry::user("pinned").with_pin(PinPosition::Top),
        ChatEntry::user("a".repeat(100)),
        ChatEntry::user("b".repeat(100)),
        ChatEntry::user("recent"),
    ];
    let strategy = make_strategy(10);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then the pinned entry survives trimming.
    assert!(result.system_prompt.is_some());
    let contents: Vec<&str> = result
        .messages
        .iter()
        .map(|m| match m {
            LlmMessage::User { content } => content.as_str(),
            other => panic!("expected User, got {other:?}"),
        })
        .collect();
    assert!(contents.contains(&"pinned"));
}

#[rstest::rstest]
#[tokio::test]
async fn pinned_entry_tokens_count_toward_compaction_budget() {
    // Given entries with a pinned entry in the middle, over threshold.
    let pinned_text = "x".repeat(100);
    let history = vec![
        ChatEntry::user("old"),
        ChatEntry::user(pinned_text.clone()).with_pin(PinPosition::Relative),
        ChatEntry::user("mid"),
        ChatEntry::user("recent"),
    ];
    let strategy = make_strategy(28);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then the pinned entry consumes budget, reducing space for unpinned.
    assert!(result.messages.len() < 4);
    let contents: Vec<&str> = result
        .messages
        .iter()
        .map(|m| match m {
            LlmMessage::User { content } => content.as_str(),
            other => panic!("expected User, got {other:?}"),
        })
        .collect();
    assert!(contents.contains(&pinned_text.as_str()));
    assert!(!contents.contains(&"old"));
}

#[rstest::rstest]
#[tokio::test]
async fn pinned_entries_survive_when_over_threshold() {
    // Given 5 entries where entry 0 and entry 2 are pinned, over threshold.
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

    // Then both pinned entries survive compaction trimming.
    assert!(result.system_prompt.is_some());
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
async fn compaction_never_splits_tool_loop() {
    // Given entries that exceed the threshold, with a tool loop at the end.
    let mut history: Vec<ChatEntry> = (0..5).map(|_| ChatEntry::user("a".repeat(400))).collect();
    history.push(ChatEntry::assistant("check"));
    history.push(ChatEntry::tool_call("call_1", "echo", "{}"));
    history.push(ChatEntry::tool_result(
        "call_1",
        "echo",
        "ok",
        ToolResultStatus::Success,
    ));
    let strategy = make_strategy(100);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then the tool-loop turn is never split.
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
    assert_eq!(has_tool_calls, has_tool_result);
}

#[rstest::rstest]
#[tokio::test]
async fn compaction_pinned_entry_forces_entire_turn() {
    // Given entries that exceed the threshold, where a ToolResult is pinned.
    let mut history: Vec<ChatEntry> = (0..5).map(|_| ChatEntry::user("a".repeat(400))).collect();
    history.push(ChatEntry::assistant("check"));
    history.push(ChatEntry::tool_call("call_1", "echo", "{}"));
    history.push(
        ChatEntry::tool_result("call_1", "echo", "ok", ToolResultStatus::Success)
            .with_pin(PinPosition::Relative),
    );
    let strategy = make_strategy(100);
    let session_id = SessionId::new();
    let context = test_context(&history, &session_id);

    // When assembling.
    let result = strategy.assemble(&context).await.expect("assemble");

    // Then the entire tool-loop turn is included (pinned ToolResult forces it).
    let messages = &result.messages;
    assert!(messages.iter().any(|m| {
        matches!(
            m,
            LlmMessage::Assistant {
                tool_calls: Some(_),
                ..
            }
        )
    }));
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, LlmMessage::Tool { .. }))
    );
}
