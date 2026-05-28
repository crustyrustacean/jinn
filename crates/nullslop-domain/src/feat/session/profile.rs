//! Per-session model and strategy selection.
//!
//! [`SessionProfile`] groups the model (LLM provider) and prompt strategy
//! for a single session. These fields are given "session priority" treatment:
//! picker selections update both the session profile and the global config,
//! while session load/save only touches the session's own profile.

use std::collections::HashSet;

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
    /// Tool names the user has explicitly disabled for this session.
    ///
    /// Opt-out model: empty set means all tools are enabled.
    /// New tools added in future versions automatically appear.
    /// Serialized as a JSON array; `#[serde(default)]` ensures legacy
    /// sessions without this field deserialize to an empty set (all enabled).
    #[serde(default)]
    pub disabled_tools: HashSet<String>,
    /// Skill names the user has explicitly disabled for this session.
    ///
    /// Opt-out model: empty set means all skills are enabled.
    /// New skills added in future versions automatically appear.
    /// `#[serde(default)]` ensures legacy sessions without this field
    /// deserialize to an empty set (all enabled).
    #[serde(default)]
    pub disabled_skills: HashSet<String>,
}

impl Default for SessionProfile {
    fn default() -> Self {
        Self {
            model: NO_PROVIDER_ID.to_owned(),
            strategy: PromptStrategyId::passthrough(),
            persona_name: DEFAULT_PERSONA_NAME.to_owned(),
            sliding_window_size: DEFAULT_SLIDING_WINDOW_SIZE,
            disabled_tools: HashSet::new(),
            disabled_skills: HashSet::new(),
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
            disabled_tools: HashSet::new(),
            disabled_skills: HashSet::new(),
        }
    }

    /// Creates a profile with all fields specified.
    pub fn new(
        model: String,
        strategy: PromptStrategyId,
        persona_name: String,
        sliding_window_size: usize,
        disabled_tools: HashSet<String>,
        disabled_skills: HashSet<String>,
    ) -> Self {
        Self {
            model,
            strategy,
            persona_name,
            sliding_window_size,
            disabled_tools,
            disabled_skills,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
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

    #[rstest::rstest]
    fn disabled_tools_round_trips_through_serde() {
        // Given a profile with disabled tools.
        let mut disabled = HashSet::new();
        disabled.insert("bash".to_owned());
        disabled.insert("edit".to_owned());
        let profile = SessionProfile::new(
            "ollama/llama3".to_owned(),
            PromptStrategyId::passthrough(),
            "coding-assistant".to_owned(),
            5,
            disabled.clone(),
            HashSet::new(),
        );

        // When serialized and deserialized.
        let json = serde_json::to_string(&profile).expect("serialize");
        let restored: SessionProfile = serde_json::from_str(&json).expect("deserialize");

        // Then disabled_tools is preserved.
        assert_eq!(restored.disabled_tools, disabled);
    }

    #[rstest::rstest]
    fn disabled_skills_round_trips_through_serde() {
        // Given a profile with disabled skills.
        let mut disabled = HashSet::new();
        disabled.insert("phased-task-loop".to_owned());
        disabled.insert("web-coder".to_owned());
        let profile = SessionProfile::new(
            "ollama/llama3".to_owned(),
            PromptStrategyId::passthrough(),
            "coding-assistant".to_owned(),
            5,
            HashSet::new(),
            disabled.clone(),
        );

        // When serialized and deserialized.
        let json = serde_json::to_string(&profile).expect("serialize");
        let restored: SessionProfile = serde_json::from_str(&json).expect("deserialize");

        // Then disabled_skills is preserved.
        assert_eq!(restored.disabled_skills, disabled);
    }

    #[rstest::rstest]
    fn legacy_json_without_disabled_skills_deserializes_to_empty_set() {
        // Given JSON from an older version that lacks disabled_skills.
        let json = r#"{"model":"ollama/llama3","strategy":"passthrough","persona_name":"coding-assistant","sliding_window_size":5,"disabled_tools":[]}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then disabled_skills is empty (all skills enabled).
        assert!(profile.disabled_skills.is_empty());
    }

    #[rstest::rstest]
    fn default_disabled_skills_is_empty() {
        let profile = SessionProfile::default();
        assert!(profile.disabled_skills.is_empty());
    }

    #[rstest::rstest]
    fn legacy_json_without_disabled_tools_deserializes_to_empty_set() {
        // Given JSON from an older version that lacks disabled_tools.
        let json = r#"{"model":"ollama/llama3","strategy":"passthrough","persona_name":"coding-assistant","sliding_window_size":5}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then disabled_tools is empty (all tools enabled).
        assert!(profile.disabled_tools.is_empty());
    }

    #[rstest::rstest]
    fn default_disabled_tools_is_empty() {
        let profile = SessionProfile::default();
        assert!(profile.disabled_tools.is_empty());
    }

    // --- Mutation-killing tests ---

    #[rstest::rstest]
    fn default_persona_name_is_coding_assistant() {
        // Given a default SessionProfile.
        let profile = SessionProfile::default();

        // Then persona_name is "coding-assistant" (not empty, not "xyzzy").
        assert_eq!(profile.persona_name, "coding-assistant");
    }

    #[rstest::rstest]
    fn default_sliding_window_size_is_5() {
        // Given a default SessionProfile.
        let profile = SessionProfile::default();

        // Then sliding_window_size is 5 (not 0, not 1).
        assert_eq!(profile.sliding_window_size, 5);
    }

    #[rstest::rstest]
    fn legacy_json_without_persona_uses_default() {
        // Given JSON from an older version that lacks persona_name.
        let json = r#"{"model":"ollama/llama3","strategy":"passthrough","sliding_window_size":5}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then persona_name defaults to "coding-assistant".
        assert_eq!(profile.persona_name, "coding-assistant");
    }

    #[rstest::rstest]
    fn legacy_json_without_sliding_window_uses_default() {
        // Given JSON from an older version that lacks sliding_window_size.
        let json = r#"{"model":"ollama/llama3","strategy":"passthrough","persona_name":"custom"}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then sliding_window_size defaults to 5.
        assert_eq!(profile.sliding_window_size, 5);
    }
}
