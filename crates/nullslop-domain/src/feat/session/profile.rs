//! Per-session model and strategy selection.
//!
//! [`SessionProfile`] groups the model (LLM provider) and prompt strategy
//! for a single session. These fields are given "session priority" treatment:
//! picker selections update both the session profile and the global config,
//! while session load/save only touches the session's own profile.

use serde::{Deserialize, Serialize};

use crate::feat::provider_infra::NO_PROVIDER_ID;
use crate::protocol::PromptStrategyId;

/// Per-session model and strategy selection.
///
/// Every session carries its own model and strategy. The session profile
/// is the single source of truth for "what model/strategy does this session use?"
/// The global config (`nullslop.toml`) holds the user's preferred defaults
/// and is updated by the picker, but session load/restore never touches it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionProfile {
    /// The model/provider for this session (e.g., "ollama/llama3").
    /// Defaults to `NO_PROVIDER_ID` — the user must select a model.
    pub model: String,
    /// The active prompt strategy for this session.
    pub strategy: PromptStrategyId,
}

impl Default for SessionProfile {
    fn default() -> Self {
        Self {
            model: NO_PROVIDER_ID.to_owned(),
            strategy: PromptStrategyId::passthrough(),
        }
    }
}

impl SessionProfile {
    /// Creates a profile seeded from config values.
    pub fn from_config(model: String, strategy: PromptStrategyId) -> Self {
        Self { model, strategy }
    }
}

#[cfg(test)]
mod tests {
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
        );

        // Then the profile uses those values.
        assert_eq!(profile.model, "ollama/llama3");
        assert_eq!(profile.strategy, PromptStrategyId::sliding_window());
    }
}
