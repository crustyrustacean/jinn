//! Per-session model, persona, and policy selection.

use serde::{Deserialize, Serialize};

use crate::{Endpoint, ModelSelection, NO_PROVIDER_ID, NameFilter, ReasoningEffort};

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
    /// Which tools this session may use.
    ///
    /// Absent means no filter: a new session starts unrestricted and an
    /// older session's missing key leaves it so. A session saved before this
    /// key existed deserializes to `None` — and so an older session's
    /// blocklist stops applying. That is the same "a stale document reads as
    /// a fresh install" stance the attendant entry and the umbrella layout
    /// take.
    ///
    /// Present with no names is not absent: it is an allow list over nothing,
    /// which permits nothing. See [`NameFilter`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_filter: Option<NameFilter>,
    /// Which skills this session may load.
    ///
    /// Replaces the former `disabled_skills` set, with the same migration
    /// story as [`Self::tool_filter`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_filter: Option<NameFilter>,
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
            tool_filter: None,
            skill_filter: None,
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
        tool_filter: Option<NameFilter>,
        skill_filter: Option<NameFilter>,
        reasoning_effort: Option<ReasoningEffort>,
        endpoint: Option<Endpoint>,
    ) -> Self {
        Self {
            model,
            persona_name,
            tool_filter,
            skill_filter,
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
    use std::collections::BTreeSet;
    use std::collections::HashSet;

    use crate::FilterMode;

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
    fn tool_filter_round_trips_through_serde() {
        // Given a profile withholding two tools by an MCP server prefix.
        let filter = NameFilter::deny(["mcp__github__*"].map(str::to_owned));
        let profile = SessionProfile::new(
            ModelSelection::Single("ollama/llama3".to_owned()),
            DEFAULT_PERSONA_NAME.to_owned(),
            Some(filter.clone()),
            None,
            None,
            None,
        );

        // When serialized and deserialized.
        let json = serde_json::to_string(&profile).expect("serialize");
        let restored: SessionProfile = serde_json::from_str(&json).expect("deserialize");

        // Then the filter survives, globs included.
        assert_eq!(restored.tool_filter, Some(filter));
    }

    #[rstest::rstest]
    fn skill_filter_round_trips_through_serde() {
        // Given a profile permitting only two skills.
        let filter = NameFilter {
            mode: FilterMode::Allow,
            names: HashSet::from(["a".to_owned(), "b".to_owned()])
                .into_iter()
                .collect(),
        };
        let profile = SessionProfile::new(
            ModelSelection::Single("ollama/llama3".to_owned()),
            DEFAULT_PERSONA_NAME.to_owned(),
            None,
            Some(filter.clone()),
            None,
            None,
        );

        // When serialized and deserialized.
        let json = serde_json::to_string(&profile).expect("serialize");
        let restored: SessionProfile = serde_json::from_str(&json).expect("deserialize");

        // Then the filter survives, mode included.
        assert_eq!(restored.skill_filter, Some(filter));
    }

    /// The point of carrying absence in the field: a present allow list over
    /// nothing is the only way to say "no tools at all", and it cannot be
    /// told apart from absence by the filter's contents alone.
    #[rstest::rstest]
    fn a_present_empty_allow_filter_round_trips_rather_than_being_dropped() {
        // Given a profile whose tool filter permits nothing.
        let filter = NameFilter {
            mode: FilterMode::Allow,
            names: BTreeSet::new(),
        };
        let profile = SessionProfile::new(
            ModelSelection::Single("ollama/llama3".to_owned()),
            DEFAULT_PERSONA_NAME.to_owned(),
            Some(filter.clone()),
            None,
            None,
            None,
        );

        // When serialized then deserialized.
        let json = serde_json::to_string(&profile).expect("serialize");
        let restored: SessionProfile = serde_json::from_str(&json).expect("deserialize");

        // Then the filter is still present, and still permits nothing.
        assert_eq!(restored.tool_filter, Some(filter));
        assert!(
            restored
                .tool_filter
                .as_ref()
                .is_some_and(|filter| !filter.permits("bash"))
        );
    }

    #[rstest::rstest]
    fn an_absent_filter_serializes_to_no_key_at_all() {
        // Given a default profile, which configures no filter.
        let profile = SessionProfile::default();

        // When serialized.
        let json = serde_json::to_string(&profile).expect("serialize");

        // Then neither key appears, so a user's file does not grow an empty
        // filter table on every save.
        assert!(!json.contains("tool_filter"), "written: {json}");
        assert!(!json.contains("skill_filter"), "written: {json}");
    }

    #[rstest::rstest]
    fn legacy_json_without_filters_permits_everything() {
        // Given JSON from a version before filters, carrying only the old keys.
        let json = r#"{"model":{"single":"ollama/llama3"},"persona_name":"coding-assistant","disabled_tools":["bash"],"disabled_skills":["web-search"]}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then both filters are absent, so nothing is withheld: the old keys
        // are unknown. This is the accepted silent failure of the breaking
        // rename.
        assert!(profile.tool_filter.is_none());
        assert!(profile.skill_filter.is_none());
    }

    #[rstest::rstest]
    fn legacy_json_without_reasoning_effort_deserializes_to_none() {
        // Given JSON from an older version that lacks reasoning_effort.
        let json = r#"{"model":{"single":"ollama/llama3"},"persona_name":"coding-assistant"}"#;

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
    fn a_default_profile_configures_no_filter() {
        // Given a default profile.
        let profile = SessionProfile::default();

        // When reading its filters.
        // Then both are absent, which at a gate permits everything — an
        // unconfigured session behaves exactly as it did before filters
        // existed.
        assert!(profile.tool_filter.is_none());
        assert!(profile.skill_filter.is_none());
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
        let json = r#"{"model":{"single":"ollama/llama3"},"persona_name":"coding-assistant","reasoning_effort":null}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then endpoint is None.
        assert!(profile.endpoint.is_none());
    }
}
