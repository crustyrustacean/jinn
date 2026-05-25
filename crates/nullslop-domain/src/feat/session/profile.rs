//! Per-session model and strategy selection.
//!
//! [`SessionProfile`] groups the model (LLM provider) and prompt strategy
//! for a single session. These fields are given "session priority" treatment:
//! picker selections update both the session profile and the global config,
//! while session load/save only touches the session's own profile.

use serde::{Deserialize, Serialize};

use crate::feat::provider_infra::NO_PROVIDER_ID;
use crate::protocol::PromptStrategyId;

/// Default persona name used when none is explicitly set.
const DEFAULT_PERSONA_NAME: &str = "coding-assistant";

/// Default sliding window size for the sliding-window strategy.
pub const DEFAULT_SLIDING_WINDOW_SIZE: usize = 5;

/// Serde default for `persona_name` — ensures old serialized sessions deserialize correctly.
fn default_persona_name() -> String {
    DEFAULT_PERSONA_NAME.to_owned()
}

fn default_sliding_window_size() -> usize {
    DEFAULT_SLIDING_WINDOW_SIZE
}

/// Per-session model, strategy, and persona selection.
///
/// Every session carries its own model, strategy, and persona. The session profile
/// is the single source of truth for "what model/strategy/persona does this session use?"
/// The global config (`nullslop.toml`) holds the user's preferred defaults
/// and is updated by the picker, but session load/restore never touches it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionProfile {
    /// The model/provider for this session (e.g., "ollama/llama3").
    /// Defaults to `NO_PROVIDER_ID` — the user must select a model.
    pub model: String,
    /// The active prompt strategy for this session.
    pub strategy: PromptStrategyId,
    /// The persona name for this session. Always populated — defaults to `"coding-assistant"`.
    /// Old serialized sessions without this field deserialize to the default.
    #[serde(default = "default_persona_name")]
    pub persona_name: String,
    #[serde(default = "default_sliding_window_size")]
    pub sliding_window_size: usize,
}

impl Default for SessionProfile {
    fn default() -> Self {
        Self {
            model: NO_PROVIDER_ID.to_owned(),
            strategy: PromptStrategyId::passthrough(),
            persona_name: DEFAULT_PERSONA_NAME.to_owned(),
            sliding_window_size: DEFAULT_SLIDING_WINDOW_SIZE,
        }
    }
}

impl SessionProfile {
    /// Creates a profile seeded from config values.
    pub fn from_config(
        model: String,
        strategy: PromptStrategyId,
        sliding_window_size: usize,
    ) -> Self {
        Self {
            model,
            strategy,
            persona_name: DEFAULT_PERSONA_NAME.to_owned(),
            sliding_window_size,
        }
    }

    /// Creates a profile with all fields specified.
    pub fn new(
        model: String,
        strategy: PromptStrategyId,
        persona_name: String,
        sliding_window_size: usize,
    ) -> Self {
        Self {
            model,
            strategy,
            persona_name,
            sliding_window_size,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    #[rstest::rstest]
    fn default_has_no_provider_and_passthrough_strategy() {
        // Given a default SessionProfile.
        let profile = SessionProfile::default();

        // Then model is NO_PROVIDER_ID and strategy is passthrough.
        assert_eq!(profile.model, NO_PROVIDER_ID);
        assert_eq!(profile.strategy, PromptStrategyId::passthrough());
    }

    #[rstest::rstest]
    fn from_config_seeds_model_and_strategy() {
        // Given config values.
        let profile = SessionProfile::from_config(
            "ollama/llama3".to_owned(),
            PromptStrategyId::sliding_window(),
            10,
        );

        // Then the profile uses those values.
        assert_eq!(profile.model, "ollama/llama3");
        assert_eq!(profile.strategy, PromptStrategyId::sliding_window());
        assert_eq!(profile.sliding_window_size, 10);
    }
}
