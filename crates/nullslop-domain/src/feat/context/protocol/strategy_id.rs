//! Strategy identification types.
//!
//! [`PromptStrategyId`] is a wire type used in commands and events to identify
//! which prompt assembly strategy a session should use.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Identifies a prompt assembly strategy.
///
/// Used as a key to look up the factory that creates the strategy instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PromptStrategyId(String);

impl PromptStrategyId {
    /// Create a new strategy ID.
    #[must_use]
    pub fn new<S>(id: S) -> Self
    where
        S: Into<String>,
    {
        Self(id.into())
    }

    /// The passthrough strategy ID.
    #[must_use]
    pub fn passthrough() -> Self {
        Self::new("passthrough")
    }

    /// The sliding window strategy ID.
    #[must_use]
    pub fn sliding_window() -> Self {
        Self::new("sliding_window")
    }

    /// The token budget strategy ID.
    #[must_use]
    pub fn token_budget() -> Self {
        Self::new("token_budget")
    }

    /// The compaction strategy ID.
    #[must_use]
    pub fn compaction() -> Self {
        Self::new("compaction")
    }

    /// The internal string identifier (e.g., `"sliding_window"`).
    ///
    /// Use this for serialization and persistence, not [`Display`].
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PromptStrategyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let display = match self.0.as_str() {
            "passthrough" => "Passthrough",
            "sliding_window" => "Sliding Window",
            "token_budget" => "Token Budget",
            "compaction" => "Compaction",
            other => other,
        };
        write!(f, "{display}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn prompt_strategy_id_passthrough_is_passthrough() {
        let id = PromptStrategyId::passthrough();
        assert_eq!(id.to_string(), "Passthrough");
    }
}
