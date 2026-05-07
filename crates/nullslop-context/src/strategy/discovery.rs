//! Default discovery of available prompt assembly strategies.
//!
//! Returns the four built-in strategies with their display metadata.

use nullslop_protocol::PromptStrategyId;

use crate::strategy::types::{StrategyDiscovery, StrategyInfo};

/// Default strategy discovery.
///
/// Returns the four built-in strategies:
/// - `passthrough` — sends history as-is
/// - `sliding_window` — keeps only the N most recent messages
/// - `token_budget` — fits messages within a token limit
/// - `compaction` — summarizes older messages into a compact form
pub struct DefaultStrategyDiscovery;

impl StrategyDiscovery for DefaultStrategyDiscovery {
    fn list(&self) -> Vec<StrategyInfo> {
        vec![
            StrategyInfo {
                id: PromptStrategyId::passthrough(),
                name: "Passthrough".to_owned(),
                description: "Send conversation history as-is, no transformation".to_owned(),
            },
            StrategyInfo {
                id: PromptStrategyId::sliding_window(),
                name: "Sliding Window".to_owned(),
                description: "Keep only the N most recent messages".to_owned(),
            },
            StrategyInfo {
                id: PromptStrategyId::token_budget(),
                name: "Token Budget".to_owned(),
                description: "Fit messages within a token limit".to_owned(),
            },
            StrategyInfo {
                id: PromptStrategyId::compaction(),
                name: "Compaction".to_owned(),
                description: "Summarize older messages into a compact form".to_owned(),
            },
        ]
    }

    fn name(&self) -> &'static str {
        "default_strategy_discovery"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_returns_four_strategies() {
        // Given the default discovery.
        let discovery = DefaultStrategyDiscovery;

        // When listing strategies.
        let strategies = discovery.list();

        // Then exactly four strategies are returned.
        assert_eq!(strategies.len(), 4);
    }

    #[test]
    fn discovery_includes_known_strategy_ids() {
        // Given the default discovery.
        let discovery = DefaultStrategyDiscovery;

        // When listing strategies.
        let strategies = discovery.list();

        // Then all four known IDs are present.
        let ids: Vec<PromptStrategyId> = strategies.iter().map(|s| s.id.clone()).collect();
        assert!(ids.contains(&PromptStrategyId::passthrough()));
        assert!(ids.contains(&PromptStrategyId::sliding_window()));
        assert!(ids.contains(&PromptStrategyId::token_budget()));
        assert!(ids.contains(&PromptStrategyId::compaction()));
    }

    #[test]
    fn discovery_strategies_have_names_and_descriptions() {
        // Given the default discovery.
        let discovery = DefaultStrategyDiscovery;

        // When listing strategies.
        let strategies = discovery.list();

        // Then every strategy has a non-empty name and description.
        for strategy in &strategies {
            assert!(!strategy.name.is_empty(), "name empty for {}", strategy.id);
            assert!(
                !strategy.description.is_empty(),
                "description empty for {}",
                strategy.id
            );
        }
    }

    #[test]
    fn discovery_name() {
        // Given the default discovery.
        let discovery = DefaultStrategyDiscovery;

        // Then it has the expected name.
        assert_eq!(discovery.name(), "default_strategy_discovery");
    }
}
