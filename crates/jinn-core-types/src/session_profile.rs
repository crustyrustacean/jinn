//! Per-session model, persona, and policy selection.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{Endpoint, ModelSelection, NO_PROVIDER_ID, ReasoningEffort};

/// Default persona name used when none is explicitly set.
pub const DEFAULT_PERSONA_NAME: &str = "coding-assistant";

/// Serde default for legacy sessions that do not contain a persona name.
fn default_persona_name() -> String {
    DEFAULT_PERSONA_NAME.to_owned()
}

/// Per-session model and persona selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionProfile {
    /// The model selection for this session, either a single model or an alloy.
    pub model: ModelSelection,
    /// The persona name for this session.
    #[serde(default = "default_persona_name")]
    pub persona_name: String,
    /// Tool names explicitly disabled for this session.
    #[serde(default)]
    pub disabled_tools: HashSet<String>,
    /// Skill names explicitly disabled for this session.
    #[serde(default)]
    pub disabled_skills: HashSet<String>,
    /// Reasoning effort selected when this session was created.
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Pinned OpenRouter routing endpoint for prefix-cache affinity.
    #[serde(default)]
    pub endpoint: Option<Endpoint>,
}

impl Default for SessionProfile {
    fn default() -> Self {
        Self {
            model: ModelSelection::Single(NO_PROVIDER_ID.to_owned()),
            persona_name: DEFAULT_PERSONA_NAME.to_owned(),
            disabled_tools: HashSet::new(),
            disabled_skills: HashSet::new(),
            reasoning_effort: None,
            endpoint: None,
        }
    }
}

impl SessionProfile {
    /// Creates a profile seeded with a single configured model.
    #[must_use]
    pub fn from_config(model: String) -> Self {
        Self::from_model_selection(ModelSelection::Single(model))
    }

    /// Creates a profile from a model selection.
    #[must_use]
    pub fn from_model_selection(model: ModelSelection) -> Self {
        Self {
            model,
            ..Self::default()
        }
    }

    /// Creates a profile with all fields explicitly specified.
    #[must_use]
    pub fn new(
        model: ModelSelection,
        persona_name: String,
        disabled_tools: HashSet<String>,
        disabled_skills: HashSet<String>,
        reasoning_effort: Option<ReasoningEffort>,
        endpoint: Option<Endpoint>,
    ) -> Self {
        Self {
            model,
            persona_name,
            disabled_tools,
            disabled_skills,
            reasoning_effort,
            endpoint,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    #[rstest::rstest]
    fn default_has_no_provider() {
        // Given a default SessionProfile.
        let profile = SessionProfile::default();

        // When reading the profile's model.
        // Then it is Single(NO_PROVIDER_ID).
        assert_eq!(
            profile.model,
            ModelSelection::Single(NO_PROVIDER_ID.to_owned())
        );
    }

    #[rstest::rstest]
    fn from_config_seeds_model() {
        // Given a model.
        let profile = SessionProfile::from_config("ollama/llama3".to_owned());

        // When reading the profile's model.
        // Then the profile uses that model.
        assert_eq!(
            profile.model,
            ModelSelection::Single("ollama/llama3".to_owned())
        );
    }

    #[rstest::rstest]
    fn disabled_tools_round_trips_through_serde() {
        // Given a profile with disabled tools.
        let disabled = HashSet::from(["bash".to_owned(), "edit".to_owned()]);
        let profile = SessionProfile::new(
            ModelSelection::Single("ollama/llama3".to_owned()),
            DEFAULT_PERSONA_NAME.to_owned(),
            disabled.clone(),
            HashSet::new(),
            None,
            None,
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
        let disabled = HashSet::from(["phased-task-loop".to_owned(), "web-coder".to_owned()]);
        let profile = SessionProfile::new(
            ModelSelection::Single("ollama/llama3".to_owned()),
            DEFAULT_PERSONA_NAME.to_owned(),
            HashSet::new(),
            disabled.clone(),
            None,
            None,
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
        let json = r#"{"model":{"single":"ollama/llama3"},"persona_name":"coding-assistant","disabled_tools":[]}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then disabled_skills is empty (all skills enabled).
        assert!(profile.disabled_skills.is_empty());
    }

    #[rstest::rstest]
    fn legacy_json_without_reasoning_effort_deserializes_to_none() {
        // Given JSON from an older version that lacks reasoning_effort.
        let json = r#"{"model":{"single":"ollama/llama3"},"persona_name":"coding-assistant","disabled_tools":[],"disabled_skills":[]}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then reasoning_effort is None.
        assert!(profile.reasoning_effort.is_none());
    }

    #[rstest::rstest]
    fn reasoning_effort_round_trips_through_serialization() {
        // Given a profile with a saved effort of High.
        let profile = SessionProfile {
            reasoning_effort: Some(ReasoningEffort::High),
            ..SessionProfile::from_config("ollama/llama3".to_owned())
        };

        // When serializing then deserializing.
        let json = serde_json::to_string(&profile).expect("serialize");
        let reloaded: SessionProfile = serde_json::from_str(&json).expect("deserialize");

        // Then the saved effort is preserved.
        assert_eq!(reloaded.reasoning_effort, Some(ReasoningEffort::High));
    }

    #[rstest::rstest]
    fn legacy_json_without_disabled_tools_deserializes_to_empty_set() {
        // Given JSON from an older version that lacks disabled_tools.
        let json = r#"{"model":{"single":"ollama/llama3"},"persona_name":"coding-assistant"}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then disabled_tools is empty (all tools enabled).
        assert!(profile.disabled_tools.is_empty());
    }

    #[rstest::rstest]
    fn default_policy_sets_are_empty() {
        // Given a default profile.
        let profile = SessionProfile::default();

        // When reading the opt-out sets.
        // Then both tool and skill sets are empty.
        assert!(profile.disabled_tools.is_empty());
        assert!(profile.disabled_skills.is_empty());
    }

    #[rstest::rstest]
    fn legacy_json_without_persona_uses_default() {
        // Given JSON from an older version that lacks persona_name.
        let json = r#"{"model":{"single":"ollama/llama3"}}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then persona_name defaults to "coding-assistant".
        assert_eq!(profile.persona_name, DEFAULT_PERSONA_NAME);
    }

    #[rstest::rstest]
    fn legacy_json_with_removed_fields_is_ignored() {
        // Given legacy JSON carrying removed strategy fields.
        let json = r#"{"model":{"single":"ollama/llama3"},"strategy":"passthrough","persona_name":"coding-assistant","sliding_window_size":5}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then known fields load normally.
        assert_eq!(
            profile.model,
            ModelSelection::Single("ollama/llama3".to_owned())
        );
        assert_eq!(profile.persona_name, DEFAULT_PERSONA_NAME);
    }

    #[rstest::rstest]
    fn endpoint_round_trips_through_serde() {
        // Given a profile with a pinned endpoint.
        let endpoint = Endpoint {
            tag: "anthropic".to_owned(),
            provider_name: "Anthropic".to_owned(),
        };
        let profile = SessionProfile {
            endpoint: Some(endpoint.clone()),
            ..SessionProfile::from_config("openrouter/anthropic/claude".to_owned())
        };

        // When serializing then deserializing.
        let json = serde_json::to_string(&profile).expect("serialize");
        let reloaded: SessionProfile = serde_json::from_str(&json).expect("deserialize");

        // Then the pinned endpoint is preserved.
        assert_eq!(reloaded.endpoint, Some(endpoint));
    }

    #[rstest::rstest]
    fn legacy_json_without_endpoint_deserializes_to_none() {
        // Given JSON from an older version that lacks endpoint.
        let json = r#"{"model":{"single":"ollama/llama3"},"persona_name":"coding-assistant","disabled_tools":[],"disabled_skills":[],"reasoning_effort":null}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then endpoint is None.
        assert!(profile.endpoint.is_none());
    }
}
