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

use crate::protocol::{ChatEntry, entries_to_messages};

use super::token_estimator::TokenEstimator;
use super::turn_grouping::{Turn, group_into_turns};
use super::types::{AssembledPrompt, AssemblyContext, PromptAssembly, PromptAssemblyError};

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

        // Group history into atomic turns.
        let turns = group_into_turns(context.history);

        // Walk turns newest → oldest, accumulating costs.
        let mut total_tokens = 0usize;
        let mut included_turns: Vec<&Turn> = Vec::new();

        for turn in turns.iter().rev() {
            let turn_tokens = turn.token_cost(self.estimator.as_ref());

            if turn.is_pinned() {
                total_tokens += turn_tokens;
                included_turns.push(turn);
                continue;
            }

            // Skip unpinned turns when budget is exceeded, but continue walking
            // to find pinned turns at older positions.
            if !included_turns.is_empty() && total_tokens + turn_tokens > effective_budget {
                continue;
            }

            total_tokens += turn_tokens;
            included_turns.push(turn);
        }

        // Reverse to chronological order and flatten to entries.
        included_turns.reverse();
        let window: Vec<ChatEntry> = included_turns
            .into_iter()
            .flat_map(|t| t.entries().cloned().collect::<Vec<_>>())
            .collect();

        let trimmed = window.len() < context.history.len();
        let messages = entries_to_messages(&window);

        Ok(AssembledPrompt {
            system_prompt: trimmed.then(|| TRIMMED_SYSTEM_PROMPT.to_owned()),
            messages,
        })
    }

    fn name(&self) -> &'static str {
        "token_budget"
    }
}
