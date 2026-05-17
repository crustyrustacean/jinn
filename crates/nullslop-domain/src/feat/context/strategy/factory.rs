//! Default factory for creating prompt assembly strategies.
//!
//! Maps [`PromptStrategyId`] values to concrete strategy instances.
//! Supports passthrough, sliding window, token budget, and compaction strategies.

use error_stack::Report;

use crate::protocol::PromptStrategyId;

use super::compaction::CompactionStrategy;
use super::passthrough::PassthroughStrategy;
use super::sliding_window::SlidingWindowStrategy;
use super::token_budget::TokenBudgetStrategy;
use super::token_estimator::CharRatioEstimator;
use super::types::{PromptAssembly, PromptAssemblyError, StrategyFactory};

/// Default sliding window size used when no configuration is provided.
const DEFAULT_SLIDING_WINDOW_SIZE: usize = 5;

/// The default strategy factory.
///
/// Creates strategies by their [`PromptStrategyId`]:
/// - `passthrough` → [`PassthroughStrategy`]
/// - `sliding_window` → [`SlidingWindowStrategy`] with default window size
pub struct DefaultStrategyFactory;

impl StrategyFactory for DefaultStrategyFactory {
    fn create(
        &self,
        id: &PromptStrategyId,
        token_budget: usize,
    ) -> Result<Box<dyn PromptAssembly>, Report<PromptAssemblyError>> {
        if id == &PromptStrategyId::passthrough() {
            Ok(Box::new(PassthroughStrategy))
        } else if id == &PromptStrategyId::sliding_window() {
            Ok(Box::new(SlidingWindowStrategy::new(
                DEFAULT_SLIDING_WINDOW_SIZE,
            )))
        } else if id == &PromptStrategyId::token_budget() {
            Ok(Box::new(TokenBudgetStrategy::new(
                token_budget,
                Box::new(CharRatioEstimator),
            )))
        } else if id == &PromptStrategyId::compaction() {
            Ok(Box::new(CompactionStrategy::new(
                token_budget,
                Box::new(CharRatioEstimator),
            )))
        } else {
            Err(Report::new(PromptAssemblyError).attach(format!("unknown strategy: {id}")))
        }
    }

    fn name(&self) -> &'static str {
        "default_strategy_factory"
    }
}

#[cfg(test)]
mod tests {
    use crate::feat::session::profile::DEFAULT_TOKEN_BUDGET;

    use super::*;

    #[rstest::rstest]
    fn factory_creates_passthrough() {
        let factory = DefaultStrategyFactory;
        let strategy = factory
            .create(&PromptStrategyId::passthrough(), DEFAULT_TOKEN_BUDGET)
            .expect("create");
        assert_eq!(strategy.name(), "passthrough");
    }

    #[rstest::rstest]
    fn factory_creates_sliding_window() {
        let factory = DefaultStrategyFactory;
        let strategy = factory
            .create(&PromptStrategyId::sliding_window(), DEFAULT_TOKEN_BUDGET)
            .expect("create");
        assert_eq!(strategy.name(), "sliding_window");
    }

    #[rstest::rstest]
    fn factory_creates_token_budget() {
        let factory = DefaultStrategyFactory;
        let strategy = factory
            .create(&PromptStrategyId::token_budget(), DEFAULT_TOKEN_BUDGET)
            .expect("create");
        assert_eq!(strategy.name(), "token_budget");
    }

    #[rstest::rstest]
    fn factory_creates_compaction() {
        let factory = DefaultStrategyFactory;
        let strategy = factory
            .create(&PromptStrategyId::compaction(), DEFAULT_TOKEN_BUDGET)
            .expect("create");
        assert_eq!(strategy.name(), "compaction");
    }

    #[rstest::rstest]
    fn factory_rejects_unknown_strategy() {
        let factory = DefaultStrategyFactory;
        let result = factory.create(&PromptStrategyId::new("nonexistent"), DEFAULT_TOKEN_BUDGET);
        assert!(result.is_err());
    }

    #[rstest::rstest]
    fn factory_name() {
        let factory = DefaultStrategyFactory;
        assert_eq!(factory.name(), "default_strategy_factory");
    }
}
