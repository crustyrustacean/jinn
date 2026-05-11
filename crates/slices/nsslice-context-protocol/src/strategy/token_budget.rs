//! Token budget strategy — limits context to fit within a token budget.
//!
//! Walks history from newest to oldest, accumulating token estimates.
//! When the budget is exceeded, older entries are dropped. Pinned entries are
//! always included regardless of budget, but their tokens still count toward
//! the accumulated total. Always includes at least the most recent entry so
//! the user's message is never lost. Sets a system prompt when context was
//! trimmed to inform the LLM.

use async_trait::async_trait;
use error_stack::Report;
use nullslop_protocol::{ChatEntry, entries_to_messages};

use crate::strategy::token_estimator::{TokenEstimator, estimate_entry_tokens};
use crate::strategy::types::{
    AssembledPrompt, AssemblyContext, PromptAssembly, PromptAssemblyError,
};

/// System prompt set when context was trimmed to fit the budget.
const TRIMMED_SYSTEM_PROMPT: &str =
    "Some earlier context was omitted to fit within the token budget.";

/// A strategy that limits context to fit within a configurable token budget.
///
/// Walks history from newest to oldest, accumulating token estimates.
/// Stops when adding another entry would exceed the budget. Always includes
/// at least the most recent entry even if it exceeds the budget.
pub struct TokenBudgetStrategy {
    /// Maximum number of tokens to include.
    max_tokens: usize,
    /// Token estimator for computing token counts.
    estimator: Box<dyn TokenEstimator>,
}

impl TokenBudgetStrategy {
    /// Create a new token budget strategy with the given budget and estimator.
    #[must_use]
    pub fn new(max_tokens: usize, estimator: Box<dyn TokenEstimator>) -> Self {
        Self {
            max_tokens,
            estimator,
        }
    }
}

#[async_trait]
impl PromptAssembly for TokenBudgetStrategy {
    async fn assemble(
        &self,
        context: &AssemblyContext<'_>,
    ) -> Result<AssembledPrompt, Report<PromptAssemblyError>> {
        if context.history.is_empty() {
            return Ok(AssembledPrompt {
                system_prompt: None,
                messages: vec![],
            });
        }

        let effective_budget = self.max_tokens.saturating_sub(context.budget_offset);

        // Walk history from newest to oldest, accumulating token estimates.
        // Pinned entries are always included regardless of budget.
        let mut included_indices = Vec::new();
        let mut total_tokens = 0usize;

        for (i, entry) in context.history.iter().enumerate().rev() {
            let entry_tokens = estimate_entry_tokens(self.estimator.as_ref(), entry);

            // Pinned entries are always included, tokens count toward budget.
            if entry.is_pinned() {
                total_tokens += entry_tokens;
                included_indices.push(i);
                continue;
            }

            // Skip unpinned entries when budget is exceeded, but continue walking
            // to find pinned entries at older indices.
            if !included_indices.is_empty() && total_tokens + entry_tokens > effective_budget {
                continue;
            }

            total_tokens += entry_tokens;
            included_indices.push(i);
        }

        included_indices.sort_unstable();

        // Collect included entries.
        let included: Vec<&ChatEntry> = included_indices
            .iter()
            .map(|&i| {
                // SAFETY: indices come from enumerate on context.history
                unsafe { context.history.get_unchecked(i) }
            })
            .collect();

        let trimmed = included.len() < context.history.len();
        let messages = entries_to_messages(&included.into_iter().cloned().collect::<Vec<_>>());

        Ok(AssembledPrompt {
            system_prompt: trimmed.then(|| TRIMMED_SYSTEM_PROMPT.to_owned()),
            messages,
        })
    }

    fn name(&self) -> &'static str {
        "token_budget"
    }
}

#[cfg(test)]
mod tests {
    use nullslop_protocol::{ChatEntry, PinPosition, SessionId};

    use super::*;
    use crate::strategy::token_estimator::CharRatioEstimator;

    fn test_context<'a>(
        history: &'a [ChatEntry],
        session_id: &'a SessionId,
    ) -> AssemblyContext<'a> {
        AssemblyContext {
            history,
            tools: &[],
            model_name: "test-model",
            session_id,
            budget_offset: 0,
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
            &nullslop_protocol::LlmMessage::User {
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
            &nullslop_protocol::LlmMessage::User {
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
                nullslop_protocol::LlmMessage::User { content } => content.as_str(),
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
                nullslop_protocol::LlmMessage::User { content } => content.as_str(),
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
                nullslop_protocol::LlmMessage::User { content } => content.as_str(),
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
            &nullslop_protocol::LlmMessage::User {
                content: "ok".to_owned(),
            }
        );
    }
}
