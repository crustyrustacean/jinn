//! Default factory for creating prompt assembly strategies.
//!
//! Maps [`StrategyConfig`] variants to concrete strategy instances.
//! Supports passthrough, sliding window, token budget, and compaction strategies.

use error_stack::Report;

use crate::protocol::PromptStrategyId;

use super::compaction::CompactionStrategy;
use super::passthrough::PassthroughStrategy;
use super::sliding_window::SlidingWindowStrategy;
use super::token_budget::TokenBudgetStrategy;
use super::token_estimator::CharRatioEstimator;
use super::types::{PromptAssembly, PromptAssemblyError, StrategyConfig, StrategyFactory};

/// The default strategy factory.
///
/// Creates strategies from [`StrategyConfig`] variants:
/// - [`StrategyConfig::Passthrough`] → [`PassthroughStrategy`]
/// - [`StrategyConfig::SlidingWindow`] → [`SlidingWindowStrategy`]
/// - [`StrategyConfig::TokenBudget`] → [`TokenBudgetStrategy`]
/// - [`StrategyConfig::Compaction`] → [`CompactionStrategy`]
pub struct DefaultStrategyFactory;

impl StrategyFactory for DefaultStrategyFactory {
    fn create(
        &self,
        _id: &PromptStrategyId,
        config: &StrategyConfig,
    ) -> Result<Box<dyn PromptAssembly>, Report<PromptAssemblyError>> {
        match config {
            StrategyConfig::Passthrough => Ok(Box::new(PassthroughStrategy)),
            StrategyConfig::SlidingWindow { window_size } => {
                Ok(Box::new(SlidingWindowStrategy::new(*window_size)))
            }
            StrategyConfig::TokenBudget { budget } => Ok(Box::new(TokenBudgetStrategy::new(
                *budget,
                Box::new(CharRatioEstimator),
            ))),
            StrategyConfig::Compaction { budget } => Ok(Box::new(CompactionStrategy::new(
                *budget,
                Box::new(CharRatioEstimator),
            ))),
        }
    }

    fn name(&self) -> &'static str {
        "default_strategy_factory"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    #[rstest::rstest]
    fn factory_creates_passthrough() {
        let factory = DefaultStrategyFactory;
        let strategy = factory
            .create(
                &PromptStrategyId::passthrough(),
                &StrategyConfig::Passthrough,
            )
            .expect("create");
        assert_eq!(strategy.name(), "passthrough");
    }

    #[rstest::rstest]
    fn factory_creates_sliding_window() {
        let factory = DefaultStrategyFactory;
        let strategy = factory
            .create(
                &PromptStrategyId::sliding_window(),
                &StrategyConfig::SlidingWindow { window_size: 5 },
            )
            .expect("create");
        assert_eq!(strategy.name(), "sliding_window");
    }

    #[rstest::rstest]
    fn factory_creates_token_budget() {
        let factory = DefaultStrategyFactory;
        let strategy = factory
            .create(
                &PromptStrategyId::token_budget(),
                &StrategyConfig::TokenBudget { budget: 150_000 },
            )
            .expect("create");
        assert_eq!(strategy.name(), "token_budget");
    }

    #[rstest::rstest]
    fn factory_creates_compaction() {
        let factory = DefaultStrategyFactory;
        let strategy = factory
            .create(
                &PromptStrategyId::compaction(),
                &StrategyConfig::Compaction { budget: 150_000 },
            )
            .expect("create");
        assert_eq!(strategy.name(), "compaction");
    }

    #[rstest::rstest]
    fn factory_name() {
        let factory = DefaultStrategyFactory;
        assert_eq!(factory.name(), "default_strategy_factory");
    }
}
