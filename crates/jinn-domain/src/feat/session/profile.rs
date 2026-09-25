//! Session-creation seed derived from user preferences.
//!
//! The portable profile value lives in `jinn-core-types`; this module keeps
//! the preferences-to-seed policy above the foundational types.

use std::collections::{BTreeSet, HashSet};

pub use jinn_core_types::{DEFAULT_PERSONA_NAME, SessionProfile};

/// Per-session defaults derived from user preferences at session creation.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSeed {
    /// Tool names to start the session with disabled.
    pub disabled_tools: HashSet<String>,
    /// Skill names to start the session with disabled.
    pub disabled_skills: HashSet<String>,
    /// MCP servers to start the session with enabled.
    pub enabled_mcp: BTreeSet<String>,
}

impl SessionSeed {
    /// Derives a new-session seed from user preferences.
    #[must_use]
    pub fn from_preferences(prefs: &jinn_preferences_config::UserPreferences) -> Self {
        Self {
            disabled_tools: prefs.disabled_tools.iter().cloned().collect(),
            disabled_skills: prefs.disabled_skills.iter().cloned().collect(),
            enabled_mcp: prefs
                .mcp_server
                .iter()
                .filter(|(_, config)| config.auto_enable)
                .map(|(name, _)| name.clone())
                .collect(),
        }
    }

    /// Returns whether the seed enables at least one MCP server.
    #[must_use]
    pub fn has_auto_enabled_mcp(&self) -> bool {
        !self.enabled_mcp.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn session_seed_from_default_preferences_is_all_enabled() {
        // Given default (empty) user preferences.
        let prefs = jinn_preferences_config::UserPreferences::default();

        // When deriving the seed.
        let seed = SessionSeed::from_preferences(&prefs);

        // Then nothing is disabled and nothing auto-enabled.
        assert!(seed.disabled_tools.is_empty());
        assert!(seed.disabled_skills.is_empty());
        assert!(!seed.has_auto_enabled_mcp());
    }

    #[rstest::rstest]
    fn session_seed_copies_preferences_and_auto_enabled_mcp() {
        // Given preferences with disabled names and one auto-enabled server.
        let prefs = jinn_preferences_config::UserPreferences {
            disabled_tools: ["bash", "mcp__excalimate__draw"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            disabled_skills: ["phased-task-loop"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            mcp_server: [(
                "excalimate".to_owned(),
                jinn_mcp_msg::McpServerConfig {
                    command: Some("npx".to_owned()),
                    auto_enable: true,
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        // When deriving the seed.
        let seed = SessionSeed::from_preferences(&prefs);

        // Then all configured defaults are copied.
        assert!(seed.disabled_tools.contains("bash"));
        assert!(seed.disabled_skills.contains("phased-task-loop"));
        assert!(seed.has_auto_enabled_mcp());
        assert_eq!(
            seed.enabled_mcp.iter().collect::<Vec<_>>(),
            vec!["excalimate"]
        );
    }

    #[rstest::rstest]
    fn session_seed_excludes_servers_without_auto_enable() {
        // Given preferences with one enabled and one disabled MCP server.
        let prefs = jinn_preferences_config::UserPreferences {
            mcp_server: [
                (
                    "on".to_owned(),
                    jinn_mcp_msg::McpServerConfig {
                        command: Some("a".to_owned()),
                        auto_enable: true,
                        ..Default::default()
                    },
                ),
                (
                    "off".to_owned(),
                    jinn_mcp_msg::McpServerConfig {
                        command: Some("b".to_owned()),
                        auto_enable: false,
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        // When deriving the seed.
        let seed = SessionSeed::from_preferences(&prefs);

        // Then only the auto-enabled server is desired.
        assert_eq!(seed.enabled_mcp.iter().collect::<Vec<_>>(), vec!["on"]);
    }
}
