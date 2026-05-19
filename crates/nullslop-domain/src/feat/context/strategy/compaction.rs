//! Compaction strategy — stub that detects when context exceeds a threshold.
//!
//! When the estimated token count of the full history is within the budget,
//! this strategy behaves like passthrough (all entries, no system prompt).
//! When it exceeds the budget, it trims newest-to-oldest (like
//! [`TokenBudgetStrategy`](super::token_budget::TokenBudgetStrategy)) and
//! sets a compaction-specific system prompt. Pinned entries are always
//! included regardless of budget, but their tokens still count toward
//! the accumulated total.
//!
//! The full implementation (LLM-based summarization, summary storage,
//! incremental compaction) is a follow-up task. This stub validates the
//! architecture and persistence plumbing.

use async_trait::async_trait;
use error_stack::Report;

use crate::protocol::{ChatEntry, entries_to_messages};

use super::token_estimator::{TokenEstimator, estimate_entry_tokens};
use super::turn_grouping::{Turn, group_into_turns};
use super::types::{AssembledPrompt, AssemblyContext, PromptAssembly, PromptAssemblyError};

/// System prompt set when context was compacted (stub: trimmed).
const COMPACTION_SYSTEM_PROMPT: &str = "Context was compacted to fit within the token budget. Earlier conversation history was summarized.";

/// A compaction strategy stub that trims context when it exceeds a token threshold.
///
/// In the full implementation, this will use LLM-based summarization instead
/// of simple trimming. For now, it falls back to token-budget-style trimming
/// with a compaction-specific system prompt.
pub struct CompactionStrategy {
    /// Maximum estimated tokens before compaction triggers.
    max_tokens: usize,
    /// Token estimator for budgeting.
    estimator: Box<dyn TokenEstimator>,
}

impl CompactionStrategy {
    /// Create a new compaction strategy with the given threshold and estimator.
    #[must_use]
    pub fn new(max_tokens: usize, estimator: Box<dyn TokenEstimator>) -> Self {
        Self {
            max_tokens,
            estimator,
        }
    }
}

#[async_trait]
impl PromptAssembly for CompactionStrategy {
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

        // Estimate total tokens across all history.
        let total_tokens: usize = context
            .history
            .iter()
            .map(|entry| estimate_entry_tokens(self.estimator.as_ref(), entry))
            .sum();

        let effective_budget = self.max_tokens.saturating_sub(context.budget_offset);

        // If everything fits, delegate to passthrough behavior.
        if total_tokens <= effective_budget {
            let messages = entries_to_messages(context.history);
            return Ok(AssembledPrompt {
                system_prompt: None,
                messages,
            });
        }

        // Over threshold — trim newest-to-oldest using turn-based walk.
        // Pinned entries are always included regardless of budget.
        let turns = group_into_turns(context.history);
        let mut included_turns: Vec<&Turn> = Vec::new();
        let mut used_tokens = 0usize;

        for turn in turns.iter().rev() {
            let turn_tokens = turn.token_cost(self.estimator.as_ref());

            // Pinned turns are always included, tokens count toward budget.
            if turn.is_pinned() {
                used_tokens += turn_tokens;
                included_turns.push(turn);
                continue;
            }

            // Skip unpinned turns when budget is exceeded, but continue walking
            // to find pinned turns at older positions.
            if !included_turns.is_empty() && used_tokens + turn_tokens > effective_budget {
                continue;
            }

            used_tokens += turn_tokens;
            included_turns.push(turn);
        }

        // Reverse to chronological order and flatten to entries.
        included_turns.reverse();
        let included: Vec<ChatEntry> = included_turns
            .into_iter()
            .flat_map(|t| t.entries().cloned().collect::<Vec<_>>())
            .collect();

        let messages = entries_to_messages(&included);

        Ok(AssembledPrompt {
            system_prompt: Some(COMPACTION_SYSTEM_PROMPT.to_owned()),
            messages,
        })
    }

    fn name(&self) -> &'static str {
        "compaction"
    }
}
