//! Per-session model and persona selection.
//!
//! [`SessionProfile`] groups the model (LLM provider) and persona
//! for a single session. These fields are given "session priority" treatment:
//! picker selections update both the session profile and the global config,
//! while session load/save only touches the session's own profile.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::feat::session::model_selection::ModelSelection;

/// Default persona name used when none is explicitly set.
const DEFAULT_PERSONA_NAME: &str = "coding-assistant";

/// Serde default for `persona_name` - ensures old serialized sessions deserialize correctly.
fn default_persona_name() -> String {
    DEFAULT_PERSONA_NAME.to_owned()
}

/// Per-session model and persona selection.
///
/// Every session carries its own model and persona. The session profile
/// is the single source of truth for "what model/persona does this session use?"
/// The global config (`jinn.toml`) holds the user's preferred defaults
/// and is updated by the picker, but session load/restore never touches it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionProfile {
    /// The model selection for this session — either a single model or an alloy.
    /// Defaults to `Single(NO_PROVIDER_ID)` — the user must select a model.
    pub model: ModelSelection,
    /// The persona name for this session. Always populated — defaults to `"coding-assistant"`.
    /// Old serialized sessions without this field deserialize to the default.
    #[serde(default = "default_persona_name")]
    pub persona_name: String,
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
            model: ModelSelection::default(),
            persona_name: DEFAULT_PERSONA_NAME.to_owned(),
            disabled_tools: HashSet::new(),
            disabled_skills: HashSet::new(),
        }
    }
}

impl SessionProfile {
    /// Creates a profile seeded from config values.
    pub fn from_config(model: String) -> Self {
        Self {
            model: ModelSelection::Single(model),
            persona_name: DEFAULT_PERSONA_NAME.to_owned(),
            disabled_tools: HashSet::new(),
            disabled_skills: HashSet::new(),
        }
    }

    /// Creates a profile from a [`ModelSelection`] (single model or alloy).
    pub fn from_model_selection(model: ModelSelection) -> Self {
        Self {
            model,
            persona_name: DEFAULT_PERSONA_NAME.to_owned(),
            disabled_tools: HashSet::new(),
            disabled_skills: HashSet::new(),
        }
    }

    /// Creates a profile with all fields specified.
    pub fn new(
        model: ModelSelection,
        persona_name: String,
        disabled_tools: HashSet<String>,
        disabled_skills: HashSet<String>,
    ) -> Self {
        Self {
            model,
            persona_name,
            disabled_tools,
            disabled_skills,
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
    use crate::feat::provider_infra::NO_PROVIDER_ID;

    use super::*;

    #[rstest::rstest]
    fn default_has_no_provider() {
        // Given a default SessionProfile.
        let profile = SessionProfile::default();

        // Then model is Single(NO_PROVIDER_ID).
        assert_eq!(
            profile.model,
            ModelSelection::Single(NO_PROVIDER_ID.to_owned())
        );
    }

    #[rstest::rstest]
    fn from_config_seeds_model() {
        // Given a model.
        let profile = SessionProfile::from_config("ollama/llama3".to_owned());

        // Then the profile uses that model.
        assert_eq!(
            profile.model,
            ModelSelection::Single("ollama/llama3".to_owned())
        );
    }

    #[rstest::rstest]
    fn disabled_tools_round_trips_through_serde() {
        // Given a profile with disabled tools.
        let mut disabled = HashSet::new();
        disabled.insert("bash".to_owned());
        disabled.insert("edit".to_owned());
        let profile = SessionProfile::new(
            ModelSelection::Single("ollama/llama3".to_owned()),
            "coding-assistant".to_owned(),
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
            ModelSelection::Single("ollama/llama3".to_owned()),
            "coding-assistant".to_owned(),
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
        let json = r#"{"model":{"single":"ollama/llama3"},"persona_name":"coding-assistant","disabled_tools":[]}"#;

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
        let json = r#"{"model":{"single":"ollama/llama3"},"persona_name":"coding-assistant"}"#;

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
    fn legacy_json_without_persona_uses_default() {
        // Given JSON from an older version that lacks persona_name.
        let json = r#"{"model":{"single":"ollama/llama3"}}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then persona_name defaults to "coding-assistant".
        assert_eq!(profile.persona_name, "coding-assistant");
    }

    #[rstest::rstest]
    fn legacy_json_with_strategy_fields_is_ignored() {
        // Given JSON from an older version that still carries strategy / sliding_window_size.
        // These fields are now removed from SessionProfile; serde must ignore them silently.
        let json = r#"{"model":{"single":"ollama/llama3"},"strategy":"passthrough","persona_name":"coding-assistant","sliding_window_size":5}"#;

        // When deserialized.
        let profile: SessionProfile = serde_json::from_str(json).expect("deserialize");

        // Then the known fields load normally.
        assert_eq!(
            profile.model,
            ModelSelection::Single("ollama/llama3".to_owned())
        );
        assert_eq!(profile.persona_name, "coding-assistant");
    }
}
