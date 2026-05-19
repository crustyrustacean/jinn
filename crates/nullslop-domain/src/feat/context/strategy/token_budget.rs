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

use super::token_estimator::{TokenEstimator, estimate_entry_tokens};
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
